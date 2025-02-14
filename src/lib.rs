mod bindings;

use bindings::exports::ntwk::theater::actor::Guest as ActorGuest;
use bindings::exports::ntwk::theater::http_server::Guest as HttpGuest;
use bindings::exports::ntwk::theater::http_server::HttpResponse;
use bindings::exports::ntwk::theater::message_server_client::Guest as MessageServerClient;
use bindings::exports::ntwk::theater::websocket_server::Guest as WebSocketGuest;
use bindings::exports::ntwk::theater::websocket_server::{
    MessageType, WebsocketMessage, WebsocketResponse,
};
use bindings::ntwk::theater::filesystem::{read_file, write_file};
use bindings::ntwk::theater::message_server_host::request;
use bindings::ntwk::theater::runtime::{log, spawn};
use bindings::ntwk::theater::types::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
struct WasmEvent {
    type_: String,
    parent: Option<u64>,
    data: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
struct InitData {
    store_id: String,
    fs_path: String,
    permissions: Vec<String>,
    websocket_port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
struct State {
    store_id: String,
    fs_proxy_id: Option<String>,
    fs_path: String,
    permissions: Vec<String>,
    head: Option<String>,
    websocket_port: u16,
    api_key: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Message {
    role: String,
    content: String,
    parent: Option<String>,
    id: Option<String>,
    fs_commands: Option<Vec<FsCommand>>,
    fs_results: Option<Vec<FsResult>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FsCommand {
    operation: String,
    path: String,
    content: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct FsResult {
    success: bool,
    operation: String,
    path: String,
    data: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum FsResponseData {
    FileList(Vec<String>),
    FileContent(String),
    None,
}

#[derive(Debug, Serialize, Deserialize)]
struct FsResponse {
    success: bool,
    data: Option<FsResponseData>,
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    _type: String,
    data: Action,
}

#[derive(Debug, Serialize, Deserialize)]
enum Action {
    Get(String),
    Put(Vec<u8>),
    All(()),
}

impl Message {
    fn new(role: String, content: String, parent: Option<String>) -> Self {
        Self {
            role,
            content,
            parent,
            id: None,
            fs_commands: None,
            fs_results: None,
        }
    }

    fn with_id(mut self, id: String) -> Self {
        self.id = Some(id);
        self
    }
}

use bindings::ntwk::theater::http_client::{send_http, HttpRequest};

// New version
impl State {
    fn resolve_path(&self, relative_path: &str) -> String {
        if relative_path.starts_with("/") {
            // Don't modify absolute paths
            relative_path.to_string()
        } else {
            // Join the base path with the relative path
            let base_path = &self.fs_path;
            if relative_path == "." {
                base_path.to_string()
            } else {
                format!("{}/{}", base_path, relative_path)
            }
        }
    }

    fn save_message(&self, msg: &Message) -> Result<String, Box<dyn std::error::Error>> {
        let req = Request {
            _type: "request".to_string(),
            data: Action::Put(serde_json::to_vec(&msg)?),
        };

        let request_bytes = serde_json::to_vec(&req)?;
        let response_bytes = request(&self.store_id, &request_bytes)?;

        let response: Value = serde_json::from_slice(&response_bytes)?;
        if response["status"].as_str() == Some("ok") {
            response["key"]
                .as_str()
                .map(|s| s.to_string())
                .ok_or("No key in response".into())
        } else {
            Err("Failed to save message".into())
        }
    }

    fn load_message(&self, id: &str) -> Result<Message, Box<dyn std::error::Error>> {
        let req = Request {
            _type: "request".to_string(),
            data: Action::Get(id.to_string()),
        };

        let request_bytes = serde_json::to_vec(&req)?;
        let response_bytes = request(&self.store_id, &request_bytes)?;

        let response: Value = serde_json::from_slice(&response_bytes)?;
        if response["status"].as_str() == Some("ok") {
            if let Some(value) = response.get("value") {
                let bytes = value
                    .as_array()
                    .ok_or("Expected byte array")?
                    .iter()
                    .map(|v| v.as_u64().unwrap_or(0) as u8)
                    .collect::<Vec<u8>>();
                let mut msg: Message = serde_json::from_slice(&bytes)?;
                msg.id = Some(id.to_string());
                return Ok(msg);
            }
        }
        Err("Failed to load message".into())
    }

    fn get_message_history(&self) -> Result<Vec<Message>, Box<dyn std::error::Error>> {
        let mut messages = Vec::new();
        let mut current_id = self.head.clone();

        while let Some(id) = current_id {
            let msg = self.load_message(&id)?;
            messages.push(msg.clone());
            current_id = msg.parent.clone();
        }

        messages.reverse(); // Oldest first
        Ok(messages)
    }

    fn generate_response(
        &self,
        messages: Vec<Message>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        log("Starting generate_response");

        // Only get preceding messages if this isn't the first message
        let command_results = if messages.len() > 1 {
            log("Getting last two messages");
            let last_messages = messages.iter().rev().take(2).collect::<Vec<_>>();

            last_messages.iter().filter_map(|msg| {
            if let Some(results) = &msg.fs_results {
                Some(format!(
                    "<command-results role=\"{}\">\n{}\n</command-results>",
                    msg.role,
                    results.iter().map(|r| format!(
                        "  <result>\n    <operation>{}</operation>\n    <path>{}</path>\n    <success>{}</success>{}{}\n  </result>",
                        r.operation,
                        r.path,
                        r.success,
                        r.data.as_ref().map(|d| format!("\n    <data>{}</data>", d)).unwrap_or_default(),
                        r.error.as_ref().map(|e| format!("\n    <error>{}</error>", e)).unwrap_or_default(),
                    )).collect::<Vec<_>>().join("\n")
                ))
            } else {
                None
            }
        }).collect::<Vec<_>>().join("\n")
        } else {
            String::new() // Empty string for the first message
        };
        log(&format!("Command results: {}", command_results));

        // Add command results to the user's message content if there are any
        let messages: Vec<AnthropicMessage> = messages
            .iter()
            .map(|msg| {
                let mut content = msg.content.clone();
                if msg.role == "user" && !command_results.is_empty() {
                    content = format!("{}\n\n{}", content, command_results);
                }
                AnthropicMessage {
                    role: msg.role.clone(),
                    content,
                }
            })
            .collect();

        // Create system message content string (not an AnthropicMessage)
        let system_content = format!(
            r#"You are a helpful AI assistant with filesystem access capabilities. Users and Assistants can interact with the filesystem using XML commands in their messages.
Available commands:
- read-file: Read contents of a file
  Example: <fs-command><operation>read-file</operation><path>example.txt</path></fs-command>

- write-file: Write content to a file
  Example: <fs-command><operation>write-file</operation><path>new.txt</path><content>Hello World</content></fs-command>

- list-files: List contents of a directory
  Example: <fs-command><operation>list-files</operation><path>.</path></fs-command>

- create-dir: Create a new directory
  Example: <fs-command><operation>create-dir</operation><path>new_folder</path></fs-command>

- delete-file: Delete a file
  Example: <fs-command><operation>delete-file</operation><path>old.txt</path></fs-command>

- delete-dir: Delete a directory
  Example: <fs-command><operation>delete-dir</operation><path>old_folder</path></fs-command>

Current filesystem root path: {}
Current permissions: {:?}

Remember to:
1. Be explicit about file operations you're suggesting
2. Use exact XML command syntax in your suggestions
3. Consider the current permissions before suggesting operations
4. Explain what each command will do before suggesting it
5. Handle operation results appropriately in follow-up messages"#,
            self.fs_path, self.permissions
        );
        log("Created system message");

        // Create messages array without system message
        let anthropic_messages: Vec<AnthropicMessage> = messages
            .iter()
            .map(|msg| AnthropicMessage {
                role: msg.role.clone(),
                content: msg.content.clone(),
            })
            .collect();
        log(&format!(
            "Created request with {} messages",
            anthropic_messages.len()
        ));

        // Create HTTP request with system as top-level parameter
        let request = HttpRequest {
            method: "POST".to_string(),
            uri: "https://api.anthropic.com/v1/messages".to_string(),
            headers: vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                ("x-api-key".to_string(), self.api_key.clone()),
                ("anthropic-version".to_string(), "2023-06-01".to_string()),
            ],
            body: Some(
                serde_json::to_vec(&json!({
                    "model": "claude-3-5-sonnet-20241022",
                    "max_tokens": 1024,
                    "system": system_content,    // Now a top-level parameter
                    "messages": anthropic_messages,
                }))
                .unwrap(),
            ),
        };
        log("Created HTTP request");

        let http_response = send_http(&request);
        log(&format!("Got HTTP response: {:?}", http_response));

        if let Some(body) = http_response.body {
            if let Ok(response_data) = serde_json::from_slice::<Value>(&body) {
                if let Some(text) = response_data["content"][0]["text"].as_str() {
                    return Ok(text.to_string());
                }
            }
        }

        Err("Failed to generate response".into())
    }

    fn allowed_operation(&self, operation: &str) -> bool {
        // read:
        // - read-file
        // - list-files
        // write:
        // - write-file
        // - create-dir
        // - delete-file
        // - delete-dir

        match operation {
            "read-file" | "list-files" => self.permissions.contains(&"read".to_string()),
            "write-file" | "create-dir" | "delete-file" | "delete-dir" => {
                self.permissions.contains(&"write".to_string())
            }
            _ => false,
        }
    }

    fn process_fs_commands(&self, commands: Vec<FsCommand>) -> Vec<FsResult> {
        let mut results = Vec::new();

        for cmd in commands {
            if !self.allowed_operation(&cmd.operation) {
                results.push(FsResult {
                    success: false,
                    operation: cmd.operation.clone(),
                    path: cmd.path,
                    data: None,
                    error: Some(format!(
                        "Operation '{}' not permitted, permitted operations: {:?}",
                        cmd.operation, self.permissions
                    )),
                });
                continue;
            }

            // Send command to fs-proxy
            if let Some(fs_proxy_id) = &self.fs_proxy_id {
                // Resolve the relative path to an absolute path
                let resolved_path = self.resolve_path(&cmd.path);
                log(&format!(
                    "Resolved path '{}' to '{}'",
                    cmd.path, resolved_path
                ));

                let req = json!({
                    "operation": cmd.operation,
                    "path": resolved_path,
                    "content": cmd.content
                });

                // In the request handling code:
                match request(fs_proxy_id, &serde_json::to_vec(&req).unwrap()) {
                    Ok(response) => {
                        log(&format!("Got response from proxy_id: {:?}", response));
                        if let Ok(resp) = serde_json::from_slice::<FsResponse>(&response) {
                            results.push(FsResult {
                                success: resp.success,
                                operation: cmd.operation.clone(),
                                path: cmd.path,
                                data: match (cmd.operation.as_str(), resp.data) {
                                    // For list-files, handle vector of strings
                                    ("list-files", Some(FsResponseData::FileList(files))) => {
                                        Some(files.join(", "))
                                    }
                                    // For read-file, handle string content
                                    ("read-file", Some(FsResponseData::FileContent(content))) => {
                                        Some(content)
                                    }
                                    // For operations that don't return data
                                    _ => None,
                                },
                                error: resp.error,
                            });
                        } else {
                            results.push(FsResult {
                                success: false,
                                operation: cmd.operation,
                                path: cmd.path,
                                data: None,
                                error: Some("Invalid response".to_string()),
                            });
                        }
                    }
                    Err(e) => {
                        results.push(FsResult {
                            success: false,
                            operation: cmd.operation,
                            path: cmd.path,
                            data: None,
                            error: Some(format!("Request failed: {}", e)),
                        });
                    }
                }
            } else {
                results.push(FsResult {
                    success: false,
                    operation: cmd.operation,
                    path: cmd.path,
                    data: None,
                    error: Some("Filesystem proxy not available".to_string()),
                });
            }
        }

        results
    }
}

impl ActorGuest for Component {
    fn init(data: Option<Json>) -> Json {
        log("Initializing filesystem chat actor");
        let data = data.unwrap();

        let init_data: InitData = serde_json::from_slice(&data).unwrap();
        log(&format!("Store actor id: {}", init_data.store_id));
        log(&format!("Filesystem path: {}", init_data.fs_path));
        log(&format!("Permissions: {:?}", init_data.permissions));
        log(&format!("Websocket port: {}", init_data.websocket_port));

        // Create and spawn fs-proxy actor
        let manifest_content = format!(
            r#"name = "fs-proxy"
version = "0.1.0"
description = "A proxy actor that provides controlled access to the filesystem"
component_path = "/Users/colinrozzi/work/actors/fs-proxy/target/wasm32-unknown-unknown/release/fs_proxy.wasm"

[interface]
implements = "ntwk:theater/actor"
requires = []

[[handlers]]
type = "runtime"
config = {{}}

[[handlers]]
type = "filesystem"
config = {{ path = "{}" }}
"#,
            init_data.fs_path
        );

        // Write manifest to data directory
        let manifest_path = "data/fs_proxy.toml";
        if let Err(e) = write_file(manifest_path, &manifest_content) {
            log(&format!("Failed to create manifest: {}", e));
            return vec![];
        }

        let full_manifest_path =
            "/Users/colinrozzi/work/actors/filesystem-chat/assets/data/fs_proxy.toml";

        // Spawn fs-proxy actor
        let fs_proxy_id = spawn(full_manifest_path);
        log(&format!("Spawned fs-proxy actor: {}", fs_proxy_id));

        // Read API key
        log("Reading API key");
        let res = read_file("api-key.txt");
        if res.is_err() {
            log("Failed to read API key");
            return vec![];
        }
        let api_key = res.unwrap();
        log("API key read");
        let api_key = String::from_utf8(api_key).unwrap().trim().to_string();
        log("API key loaded");

        let initial_state = State {
            store_id: init_data.store_id,
            fs_proxy_id: Some(fs_proxy_id),
            fs_path: init_data.fs_path,
            permissions: init_data.permissions,
            head: None,
            websocket_port: init_data.websocket_port,
            api_key,
        };

        serde_json::to_vec(&initial_state).unwrap()
    }
}

impl HttpGuest for Component {
    fn handle_request(request: HttpRequest, state: Json) -> (HttpResponse, Json) {
        let state: State = serde_json::from_slice(&state).unwrap();

        let response = match (request.method.as_str(), request.uri.as_str()) {
            ("GET", "/") | ("GET", "/index.html") => match read_file("index.html") {
                Ok(content) => HttpResponse {
                    status: 200,
                    headers: vec![("Content-Type".to_string(), "text/html".to_string())],
                    body: Some(content),
                },
                Err(e) => HttpResponse {
                    status: 500,
                    headers: vec![],
                    body: Some(format!("Failed to read index.html: {}", e).into_bytes()),
                },
            },
            ("GET", "/styles.css") => match read_file("styles.css") {
                Ok(content) => HttpResponse {
                    status: 200,
                    headers: vec![("Content-Type".to_string(), "text/css".to_string())],
                    body: Some(content),
                },
                Err(e) => HttpResponse {
                    status: 500,
                    headers: vec![],
                    body: Some(format!("Failed to read styles.css: {}", e).into_bytes()),
                },
            },
            ("GET", "/chat.js") => match read_file("chat.js") {
                Ok(content) => {
                    let str_content = String::from_utf8(content).unwrap();
                    let content = str_content
                        .replace("{{WEBSOCKET_PORT}}", &format!("{}", state.websocket_port));
                    HttpResponse {
                        status: 200,
                        headers: vec![(
                            "Content-Type".to_string(),
                            "application/javascript".to_string(),
                        )],
                        body: Some(content.into_bytes()),
                    }
                }
                Err(e) => HttpResponse {
                    status: 500,
                    headers: vec![],
                    body: Some(format!("Failed to read chat.js: {}", e).into_bytes()),
                },
            },
            ("GET", "/api/messages") => match state.get_message_history() {
                Ok(messages) => HttpResponse {
                    status: 200,
                    headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                    body: Some(
                        serde_json::to_vec(&json!({
                            "type": "message_update",
                            "messages": messages
                        }))
                        .unwrap(),
                    ),
                },
                Err(e) => HttpResponse {
                    status: 500,
                    headers: vec![],
                    body: Some(format!("Failed to load messages: {}", e).into_bytes()),
                },
            },
            _ => HttpResponse {
                status: 404,
                headers: vec![],
                body: Some(b"Not Found".to_vec()),
            },
        };

        (response, serde_json::to_vec(&state).unwrap())
    }
}

impl WebSocketGuest for Component {
    fn handle_message(message: WebsocketMessage, state: Json) -> (Json, WebsocketResponse) {
        log(&format!("Received message: {:?}", message));
        let mut state: State = serde_json::from_slice(&state).unwrap();

        match message.ty {
            MessageType::Text => {
                if let Some(text) = message.text {
                    if let Ok(command) = serde_json::from_str::<Value>(&text) {
                        match command["type"].as_str() {
                            Some("send_message") => {
                                log("Matched send_message command");
                                if let Some(content) = command["content"].as_str() {
                                    log(&format!("Processing content: {}", content));
                                    // Create initial user message
                                    let mut user_msg = Message::new(
                                        "user".to_string(),
                                        content.to_string(),
                                        state.head.clone(),
                                    );

                                    // Extract filesystem commands if present
                                    if let Some(fs_commands) = command["fs_commands"].as_array() {
                                        let commands: Vec<FsCommand> = fs_commands
                                            .iter()
                                            .filter_map(|cmd| {
                                                serde_json::from_value(cmd.clone()).ok()
                                            })
                                            .collect();

                                        if !commands.is_empty() {
                                            user_msg.fs_commands = Some(commands.clone());
                                        }
                                    }

                                    // Save the message first
                                    if let Ok(msg_id) = state.save_message(&user_msg) {
                                        log(&format!("Saved user message with id: {}", msg_id));
                                        let mut user_msg = user_msg.with_id(msg_id.clone());
                                        state.head = Some(msg_id);

                                        // Process any filesystem commands
                                        if let Some(commands) = &user_msg.fs_commands {
                                            let results =
                                                state.process_fs_commands(commands.clone());
                                            user_msg.fs_results = Some(results);

                                            // Update message with results
                                            if let Ok(updated_id) = state.save_message(&user_msg) {
                                                user_msg = user_msg.with_id(updated_id.clone());
                                                state.head = Some(updated_id);
                                            }
                                        }

                                        // Get message history and generate response
                                        log("Attempting to get message history");
                                        if let Ok(messages) = state.get_message_history() {
                                            log(&format!(
                                                "Got message history with {} messages",
                                                messages.len()
                                            ));

                                            log("Attempting to generate response");
                                            if let Ok(ai_response) =
                                                state.generate_response(messages)
                                            {
                                                log("Successfully generated AI response");

                                                // Parse fs commands from the response
                                                let mut fs_commands = Vec::new();

                                                // Simple XML parsing for fs-command tags
                                                let response_parts: Vec<&str> =
                                                    ai_response.split("<fs-command>").collect();
                                                for part in response_parts.iter().skip(1) {
                                                    if let Some(cmd_end) =
                                                        part.find("</fs-command>")
                                                    {
                                                        let cmd_xml = &part[..cmd_end];

                                                        // Extract operation
                                                        if let (Some(op_start), Some(op_end)) = (
                                                            cmd_xml.find("<operation>"),
                                                            cmd_xml.find("</operation>"),
                                                        ) {
                                                            let operation =
                                                                &cmd_xml[op_start + 11..op_end];

                                                            // Extract path
                                                            if let (
                                                                Some(path_start),
                                                                Some(path_end),
                                                            ) = (
                                                                cmd_xml.find("<path>"),
                                                                cmd_xml.find("</path>"),
                                                            ) {
                                                                let path = &cmd_xml
                                                                    [path_start + 6..path_end];

                                                                // Extract content if present
                                                                let content = if let (
                                                                    Some(content_start),
                                                                    Some(content_end),
                                                                ) = (
                                                                    cmd_xml.find("<content>"),
                                                                    cmd_xml.find("</content>"),
                                                                ) {
                                                                    Some(
                                                                        cmd_xml[content_start + 9
                                                                            ..content_end]
                                                                            .to_string(),
                                                                    )
                                                                } else {
                                                                    None
                                                                };

                                                                fs_commands.push(FsCommand {
                                                                    operation: operation
                                                                        .to_string(),
                                                                    path: path.to_string(),
                                                                    content,
                                                                });
                                                            }
                                                        }
                                                    }
                                                }

                                                // Create AI message with parsed commands
                                                let mut ai_msg = Message::new(
                                                    "assistant".to_string(),
                                                    ai_response.clone(),
                                                    Some(user_msg.id.clone().unwrap()),
                                                );

                                                // Add commands if any were found
                                                if !fs_commands.is_empty() {
                                                    ai_msg.fs_commands = Some(fs_commands.clone());

                                                    // Execute commands and store results
                                                    let results =
                                                        state.process_fs_commands(fs_commands);
                                                    ai_msg.fs_results = Some(results);
                                                }

                                                // Save AI message
                                                if let Ok(ai_msg_id) = state.save_message(&ai_msg) {
                                                    state.head = Some(ai_msg_id.clone());
                                                    let ai_msg_with_id = ai_msg.with_id(ai_msg_id);

                                                    // Send both messages
                                                    return (
                                                        serde_json::to_vec(&state).unwrap(),
                                                        WebsocketResponse {
                                                            messages: vec![WebsocketMessage {
                                                                ty: MessageType::Text,
                                                                text: Some(
                                                                    json!({
                                                                        "type": "message_update",
                                                                        "messages": [user_msg, ai_msg_with_id]
                                                                    })
                                                                    .to_string(),
                                                                ),
                                                                data: None,
                                                            }],
                                                        },
                                                    );
                                                }
                                            }
                                        }

                                        // If we get here, something went wrong, just send the user message
                                        return (
                                            serde_json::to_vec(&state).unwrap(),
                                            WebsocketResponse {
                                                messages: vec![WebsocketMessage {
                                                    ty: MessageType::Text,
                                                    text: Some(
                                                        json!({
                                                            "type": "message_update",
                                                            "message": user_msg
                                                        })
                                                        .to_string(),
                                                    ),
                                                    data: None,
                                                }],
                                            },
                                        );
                                    }
                                }
                            }
                            Some("get_messages") => {
                                if let Ok(messages) = state.get_message_history() {
                                    return (
                                        serde_json::to_vec(&state).unwrap(),
                                        WebsocketResponse {
                                            messages: vec![WebsocketMessage {
                                                ty: MessageType::Text,
                                                text: Some(
                                                    json!({
                                                        "type": "message_update",
                                                        "messages": messages
                                                    })
                                                    .to_string(),
                                                ),
                                                data: None,
                                            }],
                                        },
                                    );
                                }
                            }
                            _ => {
                                log("Unknown command type received");
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        (
            serde_json::to_vec(&state).unwrap(),
            WebsocketResponse { messages: vec![] },
        )
    }
}

impl MessageServerClient for Component {
    fn handle_send(message: Json, state: Json) -> Json {
        let mut state: State = serde_json::from_slice(&state).unwrap();

        // Attempt to parse as WasmEvent
        if let Ok(event) = serde_json::from_slice::<WasmEvent>(&message) {
            match event.type_.as_str() {
                "init" => {
                    // Handle initialization of fs-proxy
                    if let Ok(data) = String::from_utf8(event.data) {
                        log(&format!("FS-Proxy initialized: {}", data));
                    }
                }
                "terminate" => {
                    // Handle fs-proxy termination
                    if let Some(fs_proxy_id) = &state.fs_proxy_id {
                        log(&format!("FS-Proxy terminated: {}", fs_proxy_id));
                        state.fs_proxy_id = None;
                    }
                }
                _ => {
                    log(&format!("Received unknown event type: {}", event.type_));
                }
            }
        }

        serde_json::to_vec(&state).unwrap()
    }

    fn handle_request(_message: Json, state: Json) -> (Json, Json) {
        let state: State = serde_json::from_slice(&state).unwrap();

        // Handle messages that require responses
        let response = json!({
            "success": true,
            "message": "Acknowledged"
        });

        (
            serde_json::to_vec(&response).unwrap(),
            serde_json::to_vec(&state).unwrap(),
        )
    }
}

struct Component;

bindings::export!(Component with_types_in bindings);

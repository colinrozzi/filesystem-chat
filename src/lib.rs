mod bindings;

use bindings::exports::ntwk::theater::actor::Guest as ActorGuest;
use bindings::exports::ntwk::theater::http_server::Guest as HttpGuest;
use bindings::exports::ntwk::theater::http_server::{HttpRequest, HttpResponse};
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
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
struct WasmEvent {
    type_: String,
    parent: Option<u64>,
    data: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
struct State {
    fs_proxy_id: Option<String>,
    store_id: String,
    chat_sessions: HashMap<String, ChatSession>,
    message_counter: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ChatSession {
    id: String,
    fs_path: String,
    permissions: Vec<String>,
    head: Option<String>, // Points to the last message in the chain
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
struct InitData {
    store_id: String,
    websocket_port: u16,
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

impl State {
    fn save_message(&self, msg: &Message) -> Result<String, Box<dyn std::error::Error>> {
        let req = Request {
            _type: "request".to_string(),
            data: Action::Put(serde_json::to_vec(&msg)?),
        };

        let request_bytes = serde_json::to_vec(&req)?;
        let response_bytes = request(&self.store_id, &request_bytes)?;

        let response: serde_json::Value = serde_json::from_slice(&response_bytes)?;
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

        let response: serde_json::Value = serde_json::from_slice(&response_bytes)?;
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

    fn get_message_history(
        &self,
        session_id: &str,
    ) -> Result<Vec<Message>, Box<dyn std::error::Error>> {
        let session = self
            .chat_sessions
            .get(session_id)
            .ok_or("Session not found")?;

        let mut messages = Vec::new();
        let mut current_id = session.head.clone();

        while let Some(id) = current_id {
            let msg = self.load_message(&id)?;
            messages.push(msg.clone());
            current_id = msg.parent.clone();
        }

        messages.reverse(); // Oldest first
        Ok(messages)
    }

    fn update_session_head(
        &mut self,
        session_id: &str,
        message_id: String,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(session) = self.chat_sessions.get_mut(session_id) {
            session.head = Some(message_id);
            Ok(())
        } else {
            Err("Session not found".into())
        }
    }

    fn generate_id(&mut self, prefix: &str) -> String {
        self.message_counter += 1;
        format!("{}_{}", prefix, self.message_counter)
    }
}

impl ActorGuest for Component {
    fn init(data: Option<Json>) -> Json {
        log("Initializing filesystem chat actor");
        let data = data.unwrap();

        let init_data: InitData = serde_json::from_slice(&data).unwrap();
        log(&format!("Store actor id: {}", init_data.store_id));
        log(&format!("Websocket port: {}", init_data.websocket_port));

        let initial_state = State {
            fs_proxy_id: None,
            store_id: init_data.store_id,
            chat_sessions: HashMap::new(),
            message_counter: 0,
        };

        serde_json::to_vec(&initial_state).unwrap()
    }
}

impl HttpGuest for Component {
    fn handle_request(request: HttpRequest, state: Vec<u8>) -> (HttpResponse, Vec<u8>) {
        let mut state: State = serde_json::from_slice(&state).unwrap();

        let response = match (request.method.as_str(), request.uri.as_str()) {
            ("GET", "/") => match read_file("index.html") {
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
            ("GET", "/app.js") => match read_file("app.js") {
                Ok(content) => HttpResponse {
                    status: 200,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "application/javascript".to_string(),
                    )],
                    body: Some(content),
                },
                Err(e) => HttpResponse {
                    status: 500,
                    headers: vec![],
                    body: Some(format!("Failed to read app.js: {}", e).into_bytes()),
                },
            },
            ("POST", "/start-chat") => {
                match serde_json::from_slice::<StartChatRequest>(&request.body.unwrap_or_default())
                {
                    Ok(chat_request) => {
                        // Generate a unique session ID
                        let session_id = state.generate_id("chat");

                        // Create fs-proxy actor manifest with the specified path
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
                            chat_request.fs_path
                        );

                        // Write temporary manifest file
                        let manifest_path = format!("/tmp/fs_proxy_{}.toml", session_id);
                        if let Err(e) = write_file(&manifest_path, &manifest_content) {
                            let response = StartChatResponse {
                                success: false,
                                url: None,
                                error: Some(format!("Failed to create manifest: {}", e)),
                            };
                            return (
                                HttpResponse {
                                    status: 500,
                                    headers: vec![(
                                        "Content-Type".to_string(),
                                        "application/json".to_string(),
                                    )],
                                    body: Some(serde_json::to_vec(&response).unwrap()),
                                },
                                serde_json::to_vec(&state).unwrap(),
                            );
                        }

                        // Spawn the fs-proxy actor
                        let fs_proxy_id = spawn(&manifest_path);

                        // Store the session information
                        let chat_session = ChatSession {
                            id: session_id.clone(),
                            fs_path: chat_request.fs_path,
                            permissions: chat_request.permissions,
                            head: None,
                        };
                        state.chat_sessions.insert(session_id.clone(), chat_session);
                        state.fs_proxy_id = Some(fs_proxy_id);

                        // Generate the chat URL
                        let chat_url = format!("/chat/{}", session_id);

                        let response = StartChatResponse {
                            success: true,
                            url: Some(chat_url),
                            error: None,
                        };

                        HttpResponse {
                            status: 200,
                            headers: vec![(
                                "Content-Type".to_string(),
                                "application/json".to_string(),
                            )],
                            body: Some(serde_json::to_vec(&response).unwrap()),
                        }
                    }
                    Err(e) => {
                        let response = StartChatResponse {
                            success: false,
                            url: None,
                            error: Some(format!("Invalid request format: {}", e)),
                        };
                        HttpResponse {
                            status: 400,
                            headers: vec![(
                                "Content-Type".to_string(),
                                "application/json".to_string(),
                            )],
                            body: Some(serde_json::to_vec(&response).unwrap()),
                        }
                    }
                }
            }
            ("GET", uri) if uri.starts_with("/chat/") => {
                let session_id = uri.trim_start_matches("/chat/");
                if state.chat_sessions.contains_key(session_id) {
                    match read_file("chat.html") {
                        Ok(content) => {
                            let html = String::from_utf8_lossy(&content)
                                .replace("{{session_id}}", session_id);
                            HttpResponse {
                                status: 200,
                                headers: vec![(
                                    "Content-Type".to_string(),
                                    "text/html".to_string(),
                                )],
                                body: Some(html.into_bytes()),
                            }
                        }
                        Err(e) => HttpResponse {
                            status: 500,
                            headers: vec![],
                            body: Some(format!("Failed to read chat.html: {}", e).into_bytes()),
                        },
                    }
                } else {
                    HttpResponse {
                        status: 404,
                        headers: vec![],
                        body: Some(b"Chat session not found".to_vec()),
                    }
                }
            }
            _ => HttpResponse {
                status: 404,
                headers: vec![],
                body: Some(b"Not found".to_vec()),
            },
        };

        (response, serde_json::to_vec(&state).unwrap())
    }
}

impl WebSocketGuest for Component {
    fn handle_message(message: WebsocketMessage, state: Vec<u8>) -> (Vec<u8>, WebsocketResponse) {
        let mut state: State = serde_json::from_slice(&state).unwrap();

        let response = match message.ty {
            MessageType::Text => {
                if let Some(text) = message.text {
                    if let Ok(command) = serde_json::from_str::<Value>(&text) {
                        match command["type"].as_str() {
                            Some("send_message") => {
                                if let (Some(content), Some(session_id)) =
                                    (command["content"].as_str(), command["session_id"].as_str())
                                {
                                    // Get the current session
                                    if let Some(session) = state.chat_sessions.get(session_id) {
                                        // Create user message
                                        let user_msg = Message::new(
                                            "user".to_string(),
                                            content.to_string(),
                                            session.head.clone(),
                                        );

                                        // Extract any filesystem commands
                                        let fs_commands = if let Some(commands) =
                                            command["fs_commands"].as_array()
                                        {
                                            Some(
                                                commands
                                                    .iter()
                                                    .filter_map(|cmd| {
                                                        serde_json::from_value::<FsCommand>(
                                                            cmd.clone(),
                                                        )
                                                        .ok()
                                                    })
                                                    .collect::<Vec<_>>(),
                                            )
                                        } else {
                                            None
                                        };

                                        let mut user_msg_with_commands = user_msg;
                                        user_msg_with_commands.fs_commands = fs_commands.clone();

                                        // Save the message and get its ID
                                        if let Ok(msg_id) =
                                            state.save_message(&user_msg_with_commands)
                                        {
                                            if state
                                                .update_session_head(session_id, msg_id.clone())
                                                .is_ok()
                                            {
                                                let mut stored_msg =
                                                    user_msg_with_commands.with_id(msg_id.clone());

                                                // Process filesystem commands if present
                                                if let Some(commands) = fs_commands {
                                                    let mut results = Vec::new();
                                                    for cmd in commands {
                                                        let result = match cmd.operation.as_str() {
                                                            "read-file" => {
                                                                // Send request to fs-proxy
                                                                let req = json!({
                                                                    "operation": "read-file",
                                                                    "path": cmd.path
                                                                });
                                                                if let Ok(response) = request(
                                                                    state
                                                                        .fs_proxy_id
                                                                        .as_ref()
                                                                        .unwrap(),
                                                                    &serde_json::to_vec(&req)
                                                                        .unwrap(),
                                                                ) {
                                                                    if let Ok(resp) =
                                                                        serde_json::from_slice::<
                                                                            Value,
                                                                        >(
                                                                            &response
                                                                        )
                                                                    {
                                                                        if resp["success"]
                                                                            .as_bool()
                                                                            .unwrap_or(false)
                                                                        {
                                                                            FsResult {
                                                                                success: true,
                                                                                operation:
                                                                                    "read-file"
                                                                                        .to_string(),
                                                                                path: cmd.path,
                                                                                data: resp["data"]
                                                                                    .as_str()
                                                                                    .map(|s| {
                                                                                        s.to_string(
                                                                                        )
                                                                                    }),
                                                                                error: None,
                                                                            }
                                                                        } else {
                                                                            FsResult {
                                                                                success: false,
                                                                                operation:
                                                                                    "read-file"
                                                                                        .to_string(),
                                                                                path: cmd.path,
                                                                                data: None,
                                                                                error: resp
                                                                                    ["error"]
                                                                                    .as_str()
                                                                                    .map(|s| {
                                                                                        s.to_string(
                                                                                        )
                                                                                    }),
                                                                            }
                                                                        }
                                                                    } else {
                                                                        FsResult {
                                                                            success: false,
                                                                            operation: "read-file".to_string(),
                                                                            path: cmd.path,
                                                                            data: None,
                                                                            error: Some("Invalid response from fs-proxy".to_string()),
                                                                        }
                                                                    }
                                                                } else {
                                                                    FsResult {
                                                                        success: false,
                                                                        operation: "read-file".to_string(),
                                                                        path: cmd.path,
                                                                        data: None,
                                                                        error: Some("Failed to communicate with fs-proxy".to_string()),
                                                                    }
                                                                }
                                                            }
                                                            // Handle other operations similarly...
                                                            _ => FsResult {
                                                                success: false,
                                                                operation: cmd.operation,
                                                                path: cmd.path,
                                                                data: None,
                                                                error: Some(
                                                                    "Unsupported operation"
                                                                        .to_string(),
                                                                ),
                                                            },
                                                        };
                                                        results.push(result);
                                                    }

                                                    // Update the message with results
                                                    stored_msg.fs_results = Some(results);
                                                    if let Ok(updated_id) =
                                                        state.save_message(&stored_msg)
                                                    {
                                                        stored_msg = stored_msg.with_id(updated_id);
                                                    }
                                                }

                                                // Send response with the processed message
                                                return (
                                                    serde_json::to_vec(&state).unwrap(),
                                                    WebsocketResponse {
                                                        messages: vec![WebsocketMessage {
                                                            ty: MessageType::Text,
                                                            text: Some(
                                                                json!({
                                                                    "type": "message_update",
                                                                    "message": stored_msg
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
                                }
                            }
                            Some("get_messages") => {
                                if let Some(session_id) = command["session_id"].as_str() {
                                    if let Ok(messages) = state.get_message_history(session_id) {
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
                            }
                            _ => {
                                log("Unknown command type received");
                            }
                        }
                    }
                }
                WebsocketResponse {
                    messages: vec![WebsocketMessage {
                        ty: MessageType::Text,
                        text: Some(
                            json!({
                                "success": false,
                                "error": "Failed to process message"
                            })
                            .to_string(),
                        ),
                        data: None,
                    }],
                }
            }
            MessageType::Binary => WebsocketResponse { messages: vec![] },
            MessageType::Close => WebsocketResponse { messages: vec![] },
            MessageType::Connect => WebsocketResponse { messages: vec![] },
            MessageType::Ping => WebsocketResponse { messages: vec![] },
            MessageType::Pong => WebsocketResponse { messages: vec![] },
            MessageType::Other(_) => WebsocketResponse { messages: vec![] },
        };

        (serde_json::to_vec(&state).unwrap(), response)
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

#[derive(Debug, Serialize, Deserialize)]
struct StartChatRequest {
    fs_path: String,
    permissions: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StartChatResponse {
    success: bool,
    url: Option<String>,
    error: Option<String>,
}

bindings::export!(Component with_types_in bindings);

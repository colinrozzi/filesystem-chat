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
    head: Option<String>, // Points to last message in chain
    websocket_port: u16,
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
}

impl ActorGuest for Component {
    fn init(data: Option<Json>) -> Json {
        log("Initializing filesystem chat actor");
        log(&format!("Data: {:?}", data));
        let data = data.unwrap();

        log(&format!("Data: {:?}", data));

        let val = serde_json::from_slice::<Value>(&data).unwrap();
        log(&format!("Value: {:?}", val));

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

[interface]
implements = "ntwk:theater/actor"
requires = []

[[handlers]]
type = "filesystem"
config = {{ path = "{}" }}

[[handlers]]
type = "message-server"
config = {{ port = 8090 }}
interface = "ntwk:theater/message-server-client""#,
            init_data.fs_path
        );

        // Write manifest to data directory
        let manifest_path = "data/fs_proxy.toml";
        if let Err(e) = write_file(manifest_path, &manifest_content) {
            log(&format!("Failed to create manifest: {}", e));
            return vec![];
        }

        // Spawn fs-proxy actor
        let fs_proxy_id = spawn(manifest_path);
        log(&format!("Spawned fs-proxy actor: {}", fs_proxy_id));

        let initial_state = State {
            store_id: init_data.store_id,
            fs_proxy_id: Some(fs_proxy_id),
            fs_path: init_data.fs_path,
            permissions: init_data.permissions,
            head: None,
            websocket_port: init_data.websocket_port,
        };

        serde_json::to_vec(&initial_state).unwrap()
    }
}

impl HttpGuest for Component {
    fn handle_request(request: HttpRequest, state: Vec<u8>) -> (HttpResponse, Vec<u8>) {
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
                            "status": "success",
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
    fn handle_message(message: WebsocketMessage, state: Vec<u8>) -> (Vec<u8>, WebsocketResponse) {
        let mut state: State = serde_json::from_slice(&state).unwrap();

        let response = match message.ty {
            MessageType::Text => {
                if let Some(text) = message.text {
                    if let Ok(command) = serde_json::from_str::<Value>(&text) {
                        match command["type"].as_str() {
                            Some("send_message") => {
                                if let Some(content) = command["content"].as_str() {
                                    // Create user message
                                    let user_msg = Message::new(
                                        "user".to_string(),
                                        content.to_string(),
                                        state.head.clone(),
                                    );

                                    // Extract any filesystem commands
                                    let fs_commands = if let Some(commands) =
                                        command["fs_commands"].as_array()
                                    {
                                        Some(
                                            commands
                                                .iter()
                                                .filter_map(|cmd| {
                                                    serde_json::from_value::<FsCommand>(cmd.clone())
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
                                    if let Ok(msg_id) = state.save_message(&user_msg_with_commands)
                                    {
                                        state.head = Some(msg_id.clone());
                                        let mut stored_msg =
                                            user_msg_with_commands.with_id(msg_id.clone());

                                        // Process filesystem commands if present
                                        if let Some(commands) = fs_commands {
                                            let mut results = Vec::new();
                                            for cmd in commands {
                                                // Only process commands if we have permission
                                                if !state.permissions.contains(&cmd.operation) {
                                                    results.push(FsResult {
                                                        success: false,
                                                        operation: cmd.operation,
                                                        path: cmd.path,
                                                        data: None,
                                                        error: Some(
                                                            "Permission denied".to_string(),
                                                        ),
                                                    });
                                                    continue;
                                                }

                                                // Send command to fs-proxy
                                                let result =
                                                    if let Some(fs_proxy_id) = &state.fs_proxy_id {
                                                        let req = json!({
                                                            "operation": cmd.operation,
                                                            "path": cmd.path,
                                                            "content": cmd.content
                                                        });

                                                        match request(
                                                            fs_proxy_id,
                                                            &serde_json::to_vec(&req).unwrap(),
                                                        ) {
                                                            Ok(response) => {
                                                                if let Ok(resp) =
                                                                    serde_json::from_slice::<Value>(
                                                                        &response,
                                                                    )
                                                                {
                                                                    FsResult {
                                                                        success: resp["success"]
                                                                            .as_bool()
                                                                            .unwrap_or(false),
                                                                        operation: cmd.operation,
                                                                        path: cmd.path,
                                                                        data: resp["data"]
                                                                            .as_str()
                                                                            .map(|s| s.to_string()),
                                                                        error: resp["error"]
                                                                            .as_str()
                                                                            .map(|s| s.to_string()),
                                                                    }
                                                                } else {
                                                                    FsResult {
                                                                        success: false,
                                                                        operation: cmd.operation,
                                                                        path: cmd.path,
                                                                        data: None,
                                                                        error: Some(
                                                                            "Invalid response"
                                                                                .to_string(),
                                                                        ),
                                                                    }
                                                                }
                                                            }
                                                            Err(e) => FsResult {
                                                                success: false,
                                                                operation: cmd.operation,
                                                                path: cmd.path,
                                                                data: None,
                                                                error: Some(format!(
                                                                    "Request failed: {}",
                                                                    e
                                                                )),
                                                            },
                                                        }
                                                    } else {
                                                        FsResult {
                                                            success: false,
                                                            operation: cmd.operation,
                                                            path: cmd.path,
                                                            data: None,
                                                            error: Some(
                                                                "Filesystem proxy not available"
                                                                    .to_string(),
                                                            ),
                                                        }
                                                    };
                                                results.push(result);
                                            }

                                            // Update message with results
                                            stored_msg.fs_results = Some(results);
                                            if let Ok(updated_id) = state.save_message(&stored_msg)
                                            {
                                                stored_msg = stored_msg.with_id(updated_id);
                                            }
                                        }

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
                WebsocketResponse { messages: vec![] }
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

mod bindings;

use bindings::exports::ntwk::theater::actor::Guest as ActorGuest;
use bindings::exports::ntwk::theater::http_server::Guest as HttpGuest;
use bindings::exports::ntwk::theater::http_server::{HttpRequest, HttpResponse};
use bindings::exports::ntwk::theater::message_server_client::Guest as MessageServerClient;
use bindings::exports::ntwk::theater::websocket_server::Guest as WebSocketGuest;
use bindings::exports::ntwk::theater::websocket_server::{
    MessageType, WebsocketMessage, WebsocketResponse,
};
use bindings::ntwk::theater::filesystem::read_file;
use bindings::ntwk::theater::message_server_host::{request, send};
use bindings::ntwk::theater::runtime::{log, spawn};
use bindings::ntwk::theater::types::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

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

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
    session_id: String,
    fs_commands: Option<Vec<FsCommand>>,
}

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

#[derive(Debug, Serialize, Deserialize)]
struct ChatSession {
    id: String,
    fs_path: String,
    permissions: Vec<String>,
    head: Option<String>, // Points to the last message in the chain
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
    parent: Option<String>,
    id: Option<String>,
    fs_commands: Option<Vec<FsCommand>>,
    fs_results: Option<Vec<FsResult>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FsCommand {
    operation: String,
    path: String,
    content: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
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
                            chat_request.fs_path
                        );

                        // Write temporary manifest file
                        let manifest_path = format!("/tmp/fs_proxy_{}.toml", session_id);
                        if let Err(e) = std::fs::write(&manifest_path, manifest_content) {
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
        let state: State = serde_json::from_slice(&state).unwrap();

        let response = match message.ty {
            MessageType::Text => {
                if let Some(text) = message.text {
                    if let Ok(chat_message) = serde_json::from_str::<ChatMessage>(&text) {
                        // Process any filesystem commands in the message
                        if let Some(fs_commands) = chat_message.fs_commands {
                            for cmd in fs_commands {
                                log(&format!("Processing fs command: {:?}", cmd));
                                // TODO: Send commands to fs-proxy actor
                            }
                        }

                        WebsocketResponse {
                            messages: vec![WebsocketMessage {
                                ty: MessageType::Text,
                                text: Some(
                                    json!({
                                        "success": true,
                                        "messageId": format!("msg_{}", chat_message.session_id)
                                    })
                                    .to_string(),
                                ),
                                data: None,
                            }],
                        }
                    } else {
                        WebsocketResponse {
                            messages: vec![WebsocketMessage {
                                ty: MessageType::Text,
                                text: Some(
                                    json!({
                                        "success": false,
                                        "error": "Invalid message format"
                                    })
                                    .to_string(),
                                ),
                                data: None,
                            }],
                        }
                    }
                } else {
                    WebsocketResponse { messages: vec![] }
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

bindings::export!(Component with_types_in bindings);

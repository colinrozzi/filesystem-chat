mod bindings;

use bindings::exports::ntwk::theater::actor::Guest as ActorGuest;
use bindings::exports::ntwk::theater::http_server::Guest as HttpGuest;
use bindings::exports::ntwk::theater::http_server::{HttpRequest, HttpResponse};
use bindings::exports::ntwk::theater::websocket_server::Guest as WebSocketGuest;
use bindings::exports::ntwk::theater::websocket_server::{
    MessageType, WebsocketMessage, WebsocketResponse,
};
use bindings::ntwk::theater::filesystem::read_file;
use bindings::ntwk::theater::runtime::{log, spawn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

struct Component;

#[derive(Debug, Serialize, Deserialize)]
struct State {
    fs_proxy_id: Option<String>,
    chat_sessions: HashMap<String, ChatSession>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatSession {
    id: String,
    fs_path: String,
    permissions: Vec<String>,
}

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
struct FsCommand {
    operation: String,
    path: String,
    content: Option<String>,
}

impl ActorGuest for Component {
    fn init(_data: Option<Vec<u8>>) -> Vec<u8> {
        let initial_state = State {
            fs_proxy_id: None,
            chat_sessions: HashMap::new(),
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
                        let session_id = format!("chat_{}", uuid::Uuid::new_v4());

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
                                // Send command to fs-proxy actor
                                // (This will be implemented with actual message sending)
                                log(&format!("Processing fs command: {:?}", cmd));
                            }
                        }

                        // Return success response
                        WebsocketResponse {
                            messages: vec![WebsocketMessage {
                                ty: MessageType::Text,
                                text: Some(
                                    json!({
                                        "success": true,
                                        "messageId": uuid::Uuid::new_v4().to_string()
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
        };

        (serde_json::to_vec(&state).unwrap(), response)
    }
}

bindings::export!(Component with_types_in bindings);


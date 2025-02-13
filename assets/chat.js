class ChatConnection {
    constructor() {
        this.connect();
        this.messageQueue = [];
        this.retryCount = 0;
        this.maxRetries = 5;
        this.retryDelay = 1000;
        this.messageCounter = 0;
    }

    generateId(prefix = 'msg') {
        this.messageCounter++;
        return `${prefix}_${this.messageCounter}_${Date.now()}`;
    }

    connect() {
        const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        this.ws = new WebSocket(`${protocol}//${window.location.host}/ws`);
        
        this.ws.onopen = () => {
            console.log('WebSocket connected');
            this.updateStatus('Connected', true);
            this.retryCount = 0;
            
            // Send any queued messages
            while (this.messageQueue.length > 0) {
                const msg = this.messageQueue.shift();
                this.send(msg);
            }
        };

        this.ws.onclose = () => {
            console.log('WebSocket disconnected');
            this.updateStatus('Disconnected', false);
            
            if (this.retryCount < this.maxRetries) {
                setTimeout(() => {
                    this.retryCount++;
                    this.connect();
                }, this.retryDelay * Math.pow(2, this.retryCount));
            }
        };

        this.ws.onerror = (error) => {
            console.error('WebSocket error:', error);
            this.updateStatus('Error', false);
        };

        this.ws.onmessage = (event) => {
            const data = JSON.parse(event.data);
            this.handleMessage(data);
        };
    }

    updateStatus(status, connected) {
        const statusDot = document.querySelector('.status-dot');
        const statusText = document.querySelector('.status-text');
        
        statusDot.className = 'status-dot ' + (connected ? 'connected' : 'disconnected');
        statusText.textContent = status;
    }

    send(message) {
        if (this.ws.readyState === WebSocket.OPEN) {
            this.ws.send(JSON.stringify(message));
        } else {
            this.messageQueue.push(message);
        }
    }

    handleMessage(data) {
        if (data.type === 'message') {
            addMessage(data.message);
        } else if (data.type === 'fs_response') {
            handleFsResponse(data.response);
        }
    }
}

function extractFsCommands(content) {
    const parser = new DOMParser();
    const commands = [];
    
    try {
        const xmlDoc = parser.parseFromString(content, 'text/xml');
        const cmdElements = xmlDoc.getElementsByTagName('fs-command');
        
        for (const cmdElement of cmdElements) {
            const operation = cmdElement.getElementsByTagName('operation')[0]?.textContent;
            const path = cmdElement.getElementsByTagName('path')[0]?.textContent;
            const content = cmdElement.getElementsByTagName('content')[0]?.textContent;
            
            if (operation && path) {
                commands.push({
                    operation,
                    path,
                    content: content || undefined
                });
            }
        }
    } catch (error) {
        console.error('Error parsing XML commands:', error);
    }
    
    return commands;
}

function highlightXMLCommands(content) {
    // Replace XML tags with highlighted versions
    return content.replace(/<fs-command>[\s\S]*?<\/fs-command>/g, (match) => {
        return `<pre class="xml-command">${escapeHtml(match)}</pre>`;
    });
}

function escapeHtml(unsafe) {
    return unsafe
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}

function handleFsResponse(response) {
    // Add system message showing the result of filesystem operations
    const message = {
        role: 'system',
        content: response.success ? 
            `Filesystem operation completed successfully` :
            `Filesystem operation failed: ${response.error}`,
        metadata: response
    };
    
    addMessage(message);
}

function addMessage(message) {
    const container = document.getElementById('messageContainer');
    const messageDiv = document.createElement('div');
    messageDiv.className = `message ${message.role}`;
    messageDiv.id = message.id || chat.generateId('msg');
    
    let content = message.content;
    
    // Check for filesystem operations in XML format
    if (content.includes('<fs-command>')) {
        content = highlightXMLCommands(content);
    }
    
    messageDiv.innerHTML = content;
    
    // Add metadata if present
    if (message.metadata) {
        const metadataDiv = document.createElement('div');
        metadataDiv.className = 'message-metadata';
        metadataDiv.textContent = JSON.stringify(message.metadata, null, 2);
        messageDiv.appendChild(metadataDiv);
    }
    
    container.appendChild(messageDiv);
    container.scrollTop = container.scrollHeight;
}

// Set up the chat connection
const chat = new ChatConnection();

// Handle message form submission
const messageForm = document.getElementById('messageForm');
const messageInput = document.getElementById('messageInput');

messageForm.addEventListener('submit', (e) => {
    e.preventDefault();
    
    const content = messageInput.value.trim();
    if (!content) return;
    
    // Extract any filesystem commands from the message
    const fs_commands = extractFsCommands(content);
    
    // Send the message
    const message = {
        role: 'user',
        content,
        session_id: SESSION_ID,
        fs_commands: fs_commands.length > 0 ? fs_commands : undefined,
        id: chat.generateId('msg')
    };
    
    chat.send(message);
    
    // Add the message to the UI
    addMessage({
        role: 'user',
        content,
        id: message.id
    });
    
    // Clear the input
    messageInput.value = '';
    messageInput.style.height = 'auto';
});

// Handle input height adjustment
messageInput.addEventListener('input', () => {
    messageInput.style.height = 'auto';
    messageInput.style.height = messageInput.scrollHeight + 'px';
});

// Handle Shift+Enter for new lines
messageInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        messageForm.dispatchEvent(new Event('submit'));
    }
});
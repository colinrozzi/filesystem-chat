class ChatConnection {
    constructor() {
        this.connect();
        this.messageQueue = [];
        this.retryCount = 0;
        this.maxRetries = 5;
        this.retryDelay = 1000;
        this.messageCounter = 0;
    }

    connect() {
        const wsProtocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsPort = {{WEBSOCKET_PORT}}; // This gets replaced by the server
        this.ws = new WebSocket(`${wsProtocol}//${window.location.hostname}:${wsPort}`);
        
        this.ws.onopen = () => {
            console.log('WebSocket connected');
            this.updateStatus('Connected', true);
            this.retryCount = 0;
            
            // Get message history
            this.send({ type: 'get_messages' });
            
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
        if (data.type === 'message_update') {
            if (data.message) {
                // Single message update
                addMessage(data.message);
            } else if (data.messages) {
                // Full message history
                const container = document.getElementById('messageContainer');
                // Keep the system message
                const systemMessage = container.firstElementChild;
                container.innerHTML = '';
                container.appendChild(systemMessage);
                // Add all messages in order
                data.messages.forEach(addMessage);
            }
        }
    }
}

function extractFsCommands(content) {
    const parser = new DOMParser();
    const commands = [];
    
    try {
        // Add a root element to handle multiple commands
        const xmlDoc = parser.parseFromString(`<root>${content}</root>`, 'text/xml');
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

function formatFsResults(results) {
    if (!results || results.length === 0) return '';
    
    const resultElements = results.map(result => {
        const className = result.success ? 'success' : 'error';
        let content = `
            <div class="fs-result ${className}">
                <strong>${result.operation}</strong> 
                <span class="path">${result.path}</span>
        `;
        
        if (result.data) {
            content += `<div class="data">${escapeHtml(result.data)}</div>`;
        }
        
        if (result.error) {
            content += `<div class="error">${result.error}</div>`;
        }
        
        content += '</div>';
        return content;
    });
    
    return `
        <div class="fs-results">
            ${resultElements.join('')}
        </div>
    `;
}

function addMessage(message) {
    const container = document.getElementById('messageContainer');
    const messageDiv = document.createElement('div');
    messageDiv.className = `message ${message.role}`;
    
    let content = message.content;
    
    // Check for filesystem operations in XML format
    if (content.includes('<fs-command>')) {
        content = highlightXMLCommands(content);
    }
    
    messageDiv.innerHTML = content;
    
    // Add filesystem results if present
    if (message.fs_results) {
        messageDiv.innerHTML += formatFsResults(message.fs_results);
    }
    
    container.appendChild(messageDiv);
    container.scrollTop = container.scrollHeight;
}

// Set up the chat connection
const chat = new ChatConnection();

// Handle message form submission
document.addEventListener('DOMContentLoaded', () => {
    const messageForm = document.getElementById('messageForm');
    const messageInput = document.getElementById('messageInput');

    messageForm.addEventListener('submit', (e) => {
        e.preventDefault(); // Prevent form submission
        
        const content = messageInput.value.trim();
        if (!content) return;
        
        // Extract any filesystem commands
        const fs_commands = extractFsCommands(content);
        
        // Send the message
        chat.send({
            type: 'send_message',
            content,
            fs_commands: fs_commands.length > 0 ? fs_commands : undefined
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
});

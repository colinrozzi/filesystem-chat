// State management
let messageCache = new Map();
let ws = null;
let reconnectAttempts = 0;
let selectedMessageId = null;
const MAX_RECONNECT_ATTEMPTS = 5;
const WEBSOCKET_URL = 'ws://localhost:{{WEBSOCKET_PORT}}/';

// UI Elements
const messageInput = document.getElementById('messageInput');
const messageArea = document.getElementById('messageArea');
const loadingOverlay = document.getElementById('messageLoading');

// Auto-resize textarea
function adjustTextareaHeight() {
    messageInput.style.height = 'auto';
    messageInput.style.height = Math.min(messageInput.scrollHeight, 200) + 'px';
}

messageInput.addEventListener('input', adjustTextareaHeight);

// WebSocket connection management
function updateConnectionStatus(status) {
    const statusElement = document.querySelector('.connection-status');
    if (!statusElement) return;
    
    statusElement.className = 'connection-status ' + status;
    
    switch(status) {
        case 'connected':
            statusElement.textContent = 'Connected';
            break;
        case 'disconnected':
            statusElement.textContent = 'Disconnected';
            break;
        case 'connecting':
            statusElement.textContent = 'Connecting...';
            break;
    }
}

function connectWebSocket() {
    updateConnectionStatus('connecting');
    ws = new WebSocket(WEBSOCKET_URL);
    
    ws.onopen = () => {
        console.log('WebSocket connected');
        updateConnectionStatus('connected');
        reconnectAttempts = 0;
        // Request initial messages
        sendWebSocketMessage({
            type: 'get_messages'
        });
    };
    
    ws.onclose = () => {
        console.log('WebSocket disconnected');
        updateConnectionStatus('disconnected');
        if (reconnectAttempts < MAX_RECONNECT_ATTEMPTS) {
            reconnectAttempts++;
            setTimeout(connectWebSocket, 1000 * Math.min(reconnectAttempts, 30));
        }
    };
    
    ws.onerror = (error) => {
        console.error('WebSocket error:', error);
        updateConnectionStatus('disconnected');
    };
    
    ws.onmessage = (event) => {
        try {
            const data = JSON.parse(event.data);
            handleWebSocketMessage(data);
        } catch (error) {
            console.error('Error parsing WebSocket message:', error);
        }
    };
}

function sendWebSocketMessage(message) {
    if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify(message));
    } else {
        console.warn('WebSocket not connected');
        updateConnectionStatus('disconnected');
    }
}

function handleWebSocketMessage(data) {
    if (data.type === 'message_update') {
        if (data.message) {
            // Single message update
            messageCache.set(data.message.id, data.message);
            if (data.message.fs_results) {
                updateCommandResults(data.message);
            }
        } else if (data.messages) {
            // Multiple message update
            data.messages.forEach(msg => {
                messageCache.set(msg.id, msg);
                if (msg.fs_results) {
                    updateCommandResults(msg);
                }
            });
        }
        
        // Remove any temporary messages
        for (const [id, msg] of messageCache.entries()) {
            if (id.startsWith('temp-')) {
                messageCache.delete(id);
            }
        }
        
        // Render messages
        renderMessages(Array.from(messageCache.values()), false);
        
        // Update head ID
        updateHeadId(Array.from(messageCache.values()));
    }
}

function updateCommandResults(message) {
    const resultsContainer = document.querySelector('.command-results-container');
    
    if (message.fs_results && message.fs_results.length > 0) {
        const resultBlock = document.createElement('div');
        resultBlock.className = 'result-block';
        resultBlock.innerHTML = `
            <div class="result-block-header">
                <span>${message.role}'s command results</span>
                <span class="timestamp">${new Date().toLocaleTimeString()}</span>
            </div>
            <div class="result-block-content">
                ${formatFsResults(message.fs_results)}
            </div>
        `;
        
        // Add to the top of the container
        resultsContainer.insertBefore(resultBlock, resultsContainer.firstChild);
    }
}

// Update head ID in title
function updateHeadId(messages) {
    const headElement = document.querySelector('.head-id');
    if (messages && messages.length > 0) {
        const lastMessage = messages[messages.length - 1];
        headElement.textContent = `Head: ${lastMessage.id.slice(0, 8)}...`;
    } else {
        headElement.textContent = 'Head: None';
    }
}

// Message handling
async function sendMessage() {
    const text = messageInput.value.trim();
    const sendButton = document.querySelector('.send-button');

    if (!text) return;

    try {
        messageInput.disabled = true;
        sendButton.disabled = true;

        // Create temporary message for immediate display
        const tempMsg = {
            role: 'user',
            content: text,
            id: 'temp-' + Date.now(),
            parent: Array.from(messageCache.values())
                .find(msg => msg.id === selectedMessageId)?.id
        };
        messageCache.set(tempMsg.id, tempMsg);
        
        // Extract any filesystem commands
        const fs_commands = extractFsCommands(text);
        
        // Show messages with temporary one
        renderMessages([...messageCache.values()], true);

        // Send message to server
        sendWebSocketMessage({
            type: 'send_message',
            content: text,
            fs_commands: fs_commands.length > 0 ? fs_commands : undefined
        });

        // Clear input
        messageInput.value = '';
        messageInput.style.height = '2.5rem';
        messageInput.focus();
    } catch (error) {
        console.error('Error sending message:', error);
        alert('Failed to send message. Please try again.');
    } finally {
        messageInput.disabled = false;
        sendButton.disabled = false;
    }
}

// Extract filesystem commands from message
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

// Message actions
function handleMessageClick(event) {
    const messageElement = event.target.closest('.message');
    if (!messageElement) return;

    // Don't trigger if clicking action button
    if (event.target.closest('.message-action-button')) return;

    const messageId = messageElement.dataset.id;
    
    // If clicking the same message, deselect it
    if (selectedMessageId === messageId) {
        selectedMessageId = null;
    } else {
        selectedMessageId = messageId;
    }
    renderMessages([...messageCache.values()], false);
}

function copyMessageId(messageId) {
    navigator.clipboard.writeText(messageId)
        .then(() => {
            const button = document.querySelector(`[data-id="${messageId}"] .copy-button`);
            if (button) {
                const originalText = button.textContent;
                button.textContent = 'Copied!';
                setTimeout(() => {
                    button.textContent = originalText;
                }, 1000);
            }
        })
        .catch(err => {
            console.error('Failed to copy message ID:', err);
            alert('Failed to copy message ID');
        });
}

// Message formatting
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

function formatMessage(content) {
    // First escape HTML and convert newlines to <br>
    let text = escapeHtml(content).replace(/\n/g, '<br>');
    
    // Format code blocks
    text = text.replace(/```([^`]+)```/g, (match, code) => `<pre><code>${code}</code></pre>`);
    
    // Format inline code
    text = text.replace(/`([^`]+)`/g, (match, code) => `<code>${code}</code>`);
    
    // Format filesystem commands
    text = text.replace(/&lt;fs-command&gt;[\s\S]*?&lt;\/fs-command&gt;/g, (match) => {
        return `<pre class="xml-command">${match}</pre>`;
    });
    
    return text;
}

// Message rendering
function renderMessages(messages, isTyping = false) {
    // Sort messages by their sequence in the chat
    const sortedMessages = messages.sort((a, b) => {
        // If a message has a parent, it comes after that parent
        if (a.parent === b.id) return 1;
        if (b.parent === a.id) return -1;
        return 0;
    });

    // Keep the system message at the start
    const systemMessage = messageArea.querySelector('.system-message');
    
    messageArea.innerHTML = '';
    if (systemMessage) {
        messageArea.appendChild(systemMessage);
    }

    if (sortedMessages.length === 0 && !isTyping) {
        return;
    }

    const container = document.createElement('div');
    container.className = 'message-container';
    container.innerHTML = `
        ${sortedMessages.map(msg => `
            <div class="message ${msg.role} ${msg.id === selectedMessageId ? 'selected' : ''}" 
                 data-id="${msg.id}">
                ${formatMessage(msg.content)}
                <div class="message-actions">
                    <button class="message-action-button copy-button">
                        <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor">
                            <path d="M8 5H6a2 2 0 00-2 2v12a2 2 0 002 2h10a2 2 0 002-2v-1M8 5a2 2 0 002 2h2a2 2 0 002-2M8 5a2 2 0 012-2h2a2 2 0 012 2m0 0h2a2 2 0 012 2v3m2 4H10m0 0l3-3m-3 3l3 3" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
                        </svg>
                        Copy ID
                    </button>
                </div>
            </div>
        `).join('')}
        ${isTyping ? `
            <div class="typing-indicator">
                <span></span>
                <span></span>
                <span></span>
            </div>
        ` : ''}
    `;

    // Set up event listeners for messages and actions
    const messages_elements = container.querySelectorAll('.message');
    messages_elements.forEach(messageElement => {
        messageElement.addEventListener('click', handleMessageClick);
        
        const copyButton = messageElement.querySelector('.copy-button');
        if (copyButton) {
            copyButton.addEventListener('click', (e) => {
                e.stopPropagation();
                copyMessageId(messageElement.dataset.id);
            });
        }
    });

    messageArea.appendChild(container);
    messageArea.scrollTop = messageArea.scrollHeight;
}

// Utility functions
function escapeHtml(unsafe) {
    return unsafe
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}

// Event listeners
document.addEventListener('DOMContentLoaded', () => {
    connectWebSocket();

    // Setup message input handling
    messageInput.addEventListener('keydown', (event) => {
        if (event.key === 'Enter' && !event.shiftKey) {
            event.preventDefault();
            sendMessage();
        }
    });

    // Global keyboard shortcut for focusing input
    document.addEventListener('keydown', (event) => {
        if (event.key === '/' && document.activeElement !== messageInput) {
            event.preventDefault();
            messageInput.focus();
        }
    });
});

// Handle visibility changes
document.addEventListener('visibilitychange', () => {
    if (!document.hidden && (!ws || ws.readyState !== WebSocket.OPEN)) {
        connectWebSocket();
    }
});

// Cleanup on page unload
window.addEventListener('unload', () => {
    if (ws) {
        ws.close();
    }
});
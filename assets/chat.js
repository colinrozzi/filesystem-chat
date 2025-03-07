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

// Update the WebSocket message handler
function handleWebSocketMessage(data) {
    try {
        if (data.type === 'message_state_update') {
            const { message, status, last_error } = data.message_state;
            
            // Update message in cache
            messageCache.set(message.id, message);
            
            // Store processing state
            processingStates.set(message.id, {
                status,
                lastError: last_error
            });
            
            renderMessages([...messageCache.values()], false);
        } else if (data.type === 'message_update') {
            // Handle bulk message updates (e.g., from get_messages)
            if (data.messages) {
                data.messages.forEach(msg => {
                    messageCache.set(msg.id, msg);
                });
            }
            renderMessages([...messageCache.values()], false);
        }
    } catch (error) {
        console.error('Error handling WebSocket message:', error);
    }
}

function updateCommandResults(message) {
    const resultsContainer = document.querySelector('.command-results-container');
    
    if (message.fs_results && message.fs_results.length > 0) {
        const resultBlock = document.createElement('div');
        resultBlock.className = 'result-block';
        // Extract the command from the message content
        let command = "Command";
        const commandMatch = message.content.match(/<fs-command>[\s\S]*?<\/fs-command>/g);
        if (commandMatch) {
            // Clean up the command for display
            // Extract operation and path separately
            const operation = commandMatch[0].match(/<operation>([^<]+)<\/operation>/)?.[1] || '';
            const path = commandMatch[0].match(/<path>([^<]+)<\/path>/)?.[1] || '';
            const content = commandMatch[0].match(/<content>([^<]+)<\/content>/)?.[1];
            
            // Combine them with proper spacing
            command = `${operation} ${path}`;
            if (content) {
                command += ` (with content)`;
            }
        }

        resultBlock.innerHTML = `
            <div class="result-block-header">
                <div class="header-content">
                    <span class="command-text">${escapeHtml(command)}</span>
                    <span class="command-meta">${message.role} • ${new Date().toLocaleTimeString()}</span>
                </div>
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

// Handle info pane collapse
function setupInfoPane() {
    const infoPane = document.querySelector('.info-pane');
    const collapseButton = document.getElementById('collapseButton');

    collapseButton.addEventListener('click', () => {
        infoPane.classList.toggle('collapsed');
    });
}

// Handle command result collapse
function handleResultBlockClick(event) {
    const header = event.target.closest('.result-block-header');
    if (header) {
        const block = header.closest('.result-block');
        block.classList.toggle('expanded');
    }
}

// Add to the existing state management
let processingStates = new Map();
let retryTimeouts = new Map();

// Add these new message handling functions
function handleMessageState(messageState) {
    const { message, status, last_error, next_retry } = messageState;
    
    // Update the message cache
    messageCache.set(message.id, message);
    
    // Update processing state
    processingStates.set(message.id, {
        status,
        lastError: last_error,
        nextRetry: next_retry
    });
    
    // Handle retries
    if (status === 'RetryScheduled' && next_retry) {
        scheduleRetry(message.id, next_retry);
    }
    
    // Update UI
    renderMessages([...messageCache.values()], false);
}

function scheduleRetry(messageId, retryTime) {
    // Clear any existing retry timeout
    if (retryTimeouts.has(messageId)) {
        clearTimeout(retryTimeouts.get(messageId));
    }
    
    // Calculate delay in milliseconds
    const now = Math.floor(Date.now() / 1000);
    const delay = (retryTime - now) * 1000;
    
    // Schedule retry
    const timeout = setTimeout(() => {
        retryMessage(messageId);
        retryTimeouts.delete(messageId);
    }, delay);
    
    retryTimeouts.set(messageId, timeout);
}

function retryMessage(messageId) {
    sendWebSocketMessage({
        type: 'retry_message',
        messageId
    });
}

// Update the message rendering to show status and errors
function renderMessage(msg) {
    const processState = processingStates.get(msg.id);
    const isProcessing = processState?.status === 'ProcessingCommands' 
        || processState?.status === 'GeneratingResponse';
    const hasFailed = processState?.status === 'Failed';
    
    return `
        <div class="message ${msg.role} ${msg.id === selectedMessageId ? 'selected' : ''} 
                          ${isProcessing ? 'processing' : ''} 
                          ${hasFailed ? 'failed' : ''}"
             data-id="${msg.id}">
            ${formatMessage(msg.content)}
            
            ${hasFailed ? `
                <div class="error-banner">
                    <span class="error-message">${processState.lastError || 'An error occurred'}</span>
                    ${msg.retries < 3 ? `
                        <button class="retry-button" onclick="retryMessage('${msg.id}')">
                            Retry
                        </button>
                    ` : ''}
                </div>
            ` : ''}
            
            ${isProcessing ? `
                <div class="processing-indicator">
                    <span></span><span></span><span></span>
                </div>
            ` : ''}
            
            <div class="message-actions">
                <button class="message-action-button copy-button" onclick="copyMessageId('${msg.id}')">
                    Copy ID
                </button>
            </div>
        </div>
    `;
}


function formatRetryTime(timestamp) {
    const now = Math.floor(Date.now() / 1000);
    const seconds = timestamp - now;
    
    if (seconds < 60) {
        return `${seconds} seconds`;
    } else {
        return `${Math.floor(seconds / 60)} minutes`;
    }
}

// Event listeners
document.addEventListener('DOMContentLoaded', () => {
    connectWebSocket();
    setupInfoPane();

    // Make sure Available Commands section starts expanded
    const commandsSection = document.querySelector('.info-section.result-block');
    commandsSection.classList.add('expanded');
    
    // Add click handler for result blocks and Available Commands
    document.querySelector('.info-pane-content').addEventListener('click', handleResultBlockClick);

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

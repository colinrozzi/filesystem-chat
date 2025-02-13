// Recent chats management
const MAX_RECENT_CHATS = 5;

function loadRecentChats() {
    const chats = JSON.parse(localStorage.getItem('recentFsChats') || '[]');
    const recentChatsList = document.getElementById('recentChatsList');
    
    if (chats.length === 0) {
        recentChatsList.innerHTML = '<div class="no-chats-message">No recent chats</div>';
        return;
    }
    
    recentChatsList.innerHTML = chats
        .sort((a, b) => b.timestamp - a.timestamp)
        .map(chat => {
            const date = new Date(chat.timestamp);
            const timeString = date.toLocaleString();
            return `
                <div class="chat-item">
                    <a href="${chat.url}" target="_blank">${chat.fsPath}</a>
                    <span class="timestamp">${timeString}</span>
                    <span class="permissions">${chat.permissions.join(', ')}</span>
                </div>
            `;
        })
        .join('');
}

function addRecentChat(fsPath, url, permissions) {
    const chats = JSON.parse(localStorage.getItem('recentFsChats') || '[]');
    
    // Add new chat to the beginning
    chats.unshift({
        fsPath,
        url,
        permissions,
        timestamp: Date.now()
    });
    
    // Keep only the most recent chats
    const recentChats = chats.slice(0, MAX_RECENT_CHATS);
    
    localStorage.setItem('recentFsChats', JSON.stringify(recentChats));
    loadRecentChats();
}

// Clear recent chats
document.getElementById('clearRecentChats').addEventListener('click', () => {
    if (confirm('Are you sure you want to clear your recent chats history?')) {
        localStorage.removeItem('recentFsChats');
        loadRecentChats();
    }
});

// Load recent chats on page load
document.addEventListener('DOMContentLoaded', loadRecentChats);

// Form submission handler
document.getElementById('fsForm').addEventListener('submit', async (e) => {
    e.preventDefault();
    
    const fsPath = document.getElementById('fsPath').value;
    const permissions = Array.from(document.querySelectorAll('input[name="permissions"]:checked'))
        .map(input => input.value);

    if (permissions.length === 0) {
        alert('Please select at least one permission');
        return;
    }

    const data = {
        fs_path: fsPath,
        permissions: permissions
    };

    try {
        const response = await fetch('/start-chat', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
                'Accept': 'application/json',
            },
            body: JSON.stringify(data)
        });

        if (!response.ok) {
            const errorText = await response.text();
            throw new Error(errorText || 'Failed to start chat');
        }

        const result = await response.json();
        
        // Show the result
        const resultDiv = document.getElementById('result');
        const chatLink = document.getElementById('chatLink');
        
        chatLink.href = result.url;
        chatLink.textContent = result.url;
        resultDiv.classList.remove('hidden');

        // Add to recent chats
        addRecentChat(fsPath, result.url, permissions);

    } catch (error) {
        console.error('Error starting chat:', error);
        alert('Failed to start chat: ' + error.message);
    }
});
// Teambook Health Monitor - Enhanced JavaScript with AI Node Visualization

// Real code snippets for the data stream effect (matching website)
const codeSnippets = [
    "import { DuckDBClient } from 'duckdb';",
    "async function remember(content, tags) {",
    "const embeddings = await generateVector(text);",
    "SELECT * FROM notes WHERE created > NOW() - INTERVAL '7 days';",
    "conn.execute('CREATE TABLE IF NOT EXISTS memories');",
    "export const world = { datetime, location, weather };",
    "function parseTimeQuery(when) { return moment(when); }",
    "await redis.publish('teambook:events', JSON.stringify(msg));",
    "const ai_id = crypto.randomUUID().slice(0, 8);",
    "return vectorStore.similaritySearch(query, k=5);",
    "if (priority === 'high') queue.unshift(task);",
    "const pagerank = calculateGraphRank(connections);",
    "await postgres.query('INSERT INTO teambook VALUES ($1, $2)');",
    "fs.writeFileSync(notebookPath, JSON.stringify(data));",
    "class TaskManager extends MCPServer {",
    "session.set('last_operation', { type, result });",
    "const weather = await fetch(`https://api.open-meteo.com`);",
    "function smartTruncate(text, maxChars) {",
    "import Papa from 'papaparse'; // CSV parsing",
    "const db = await DuckDB.connect(':memory:');",
    "logger.info('Teambook v1.0.0 initialized');",
    "return notes.filter(n => n.pinned).sort(byDate);",
    "async function broadcast(channel, message) {",
    "const location = await ipToLocation(ipAddress);",
    "tool.register('remember', notebookRemember);",
    "export default { notebook, task, teambook, world };",
    "const redis = new Redis({ host: 'localhost', port: 6379 });",
    "function detectPriority(text) { /* NLP logic */ }",
    "await standby_mode({ timeout: 180, wakeOn: ['dm', 'mention'] });",
    "const tokens = text.split(/\\s+/).length;",
    "subscriber.on('message', handleEvent);",
    "return similarity > 0.7 ? matches : [];",
    "function formatTimeContextual(timestamp) {",
    "CREATE INDEX idx_embeddings ON notes USING hnsw(embedding);",
    "const tasks = db.prepare('SELECT * FROM tasks WHERE status = ?');",
    "if (ctx.assignee === getCurrentAI()) return task;",
    "await notebook.pin(noteId); // Mark as important",
    "const semanticResults = vectorDB.query(embedding);",
    "export const VERSION = '1.0.0';",
    "function logOperation(op, duration) { stats[op] = duration; }",
    "const pageRank = edges.reduce((acc, e) => acc + e.weight, 0);",
    "await teambook.write({ content, summary, tags });",
    "class OperationTracker { constructor(toolName) {} }",
    "const now = moment().tz('Australia/Melbourne');",
    "return results.map(r => ({ id: r.note_id, score: r.similarity }));",
    "async function evolve(goal, output) { /* Multi-AI collaboration */ }",
    "const lock = await acquireLock(resourceId, timeout);",
    "pub.publish('channel', JSON.stringify({ event: 'note_created' }));",
    "function parseNaturalLanguage(query) { return chronoParse(query); }",
    "const path = require('path').join(__dirname, 'data', 'notebook.db');"
];

// Data stream effect with falling code (matching website aesthetic)
function initDataStream() {
    const dataStream = document.getElementById('dataStream');
    // 3x more columns for tighter spacing
    const columns = Math.floor(window.innerWidth / 80) * 3;
    
    // Create falling code columns
    for (let i = 0; i < columns; i++) {
        createColumn(i);
    }
    
    function createColumn(index) {
        const column = document.createElement('div');
        column.className = 'data-column';
        
        // Random horizontal position
        column.style.left = (Math.random() * window.innerWidth) + 'px';
        
        // Pick random code snippet
        const codeSnippet = codeSnippets[Math.floor(Math.random() * codeSnippets.length)];
        column.textContent = codeSnippet;
        
        // Random animation duration (12-25 seconds)
        const duration = (Math.random() * 13 + 12) + 's';
        column.style.animationDuration = duration;
        
        // Random delay for staggered start
        const delay = (Math.random() * 10) + 's';
        column.style.animationDelay = delay;
        
        dataStream.appendChild(column);
        
        // Remove and recreate after animation completes
        const totalDuration = (parseFloat(duration) + parseFloat(delay)) * 1000;
        setTimeout(() => {
            column.remove();
            createColumn(index);
        }, totalDuration);
    }
}

// API endpoint (will be served by Python backend)
const API_BASE = 'http://localhost:8765/api';

// WebSocket for real-time teambook monitoring
let wsConnection = null;
let reconnectInterval = null;

// State
let healthData = {
    postgresql: null,
    redis: null,
    duckdb: null,
    activeBackend: 'unknown',
    stats: {},
    activeAIs: [] // For AI node visualization
};

// Initialize on page load
document.addEventListener('DOMContentLoaded', () => {
    initDataStream();
    initAINodeGraph();
    checkAllBackends();
    connectWebSocket();
    startAutoRefresh();
});

// WebSocket connection for real-time teambook updates
function connectWebSocket() {
    try {
        wsConnection = new WebSocket('ws://localhost:8765/ws/teambook');
        
        wsConnection.onopen = () => {
            console.log('✓ WebSocket connected - real-time monitoring active');
            if (reconnectInterval) {
                clearInterval(reconnectInterval);
                reconnectInterval = null;
            }
        };
        
        wsConnection.onmessage = (event) => {
            const data = JSON.parse(event.data);
            handleRealtimeUpdate(data);
        };
        
        wsConnection.onerror = (error) => {
            console.error('WebSocket error:', error);
        };
        
        wsConnection.onclose = () => {
            console.log('WebSocket disconnected - attempting reconnect...');
            // Attempt reconnection every 5 seconds
            if (!reconnectInterval) {
                reconnectInterval = setInterval(connectWebSocket, 5000);
            }
        };
    } catch (error) {
        console.error('Failed to establish WebSocket connection:', error);
    }
}

// Handle real-time updates from teambook
function handleRealtimeUpdate(data) {
    if (data.type === 'ai_status') {
        healthData.activeAIs = data.ais;
        updateAINodeGraph(data.ais);
    } else if (data.type === 'note_created') {
        // Flash notification or update stats
        updateStats(data.stats);
    } else if (data.type === 'backend_change') {
        // Backend switched
        updateFallbackChain(data.activeBackend);
    }
}

// Initialize AI Node Graph (Cyberpunk style)
function initAINodeGraph() {
    const container = document.getElementById('ai-network-container');
    if (!container) {
        console.warn('AI network container not found - skipping visualization');
        return;
    }
    
    // Create SVG canvas for network graph
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('width', '100%');
    svg.setAttribute('height', '400');
    svg.setAttribute('id', 'ai-network-svg');
    container.appendChild(svg);
    
    // Add initial placeholder
    const text = document.createElementNS('http://www.w3.org/2000/svg', 'text');
    text.setAttribute('x', '50%');
    text.setAttribute('y', '50%');
    text.setAttribute('text-anchor', 'middle');
    text.setAttribute('fill', '#82A473');
    text.setAttribute('font-family', 'JetBrains Mono');
    text.setAttribute('font-size', '14');
    text.textContent = 'Waiting for AI connections...';
    svg.appendChild(text);
}

// Update AI Node Graph with current AI statuses
function updateAINodeGraph(ais) {
    const svg = document.getElementById('ai-network-svg');
    if (!svg) return;
    
    // Clear existing content
    while (svg.firstChild) {
        svg.removeChild(svg.firstChild);
    }
    
    if (!ais || ais.length === 0) {
        const text = document.createElementNS('http://www.w3.org/2000/svg', 'text');
        text.setAttribute('x', '50%');
        text.setAttribute('y', '50%');
        text.setAttribute('text-anchor', 'middle');
        text.setAttribute('fill', '#878787');
        text.setAttribute('font-family', 'JetBrains Mono');
        text.setAttribute('font-size', '14');
        text.textContent = 'No active AIs detected';
        svg.appendChild(text);
        return;
    }
    
    // Calculate positions for nodes in a circle
    const centerX = svg.clientWidth / 2;
    const centerY = 200;
    const radius = 120;
    
    // Draw connections first (so they appear behind nodes)
    const g = document.createElementNS('http://www.w3.org/2000/svg', 'g');
    g.setAttribute('id', 'connections');
    svg.appendChild(g);
    
    // Draw connection lines between all AIs
    for (let i = 0; i < ais.length; i++) {
        for (let j = i + 1; j < ais.length; j++) {
            const angle1 = (i / ais.length) * 2 * Math.PI - Math.PI / 2;
            const angle2 = (j / ais.length) * 2 * Math.PI - Math.PI / 2;
            
            const x1 = centerX + radius * Math.cos(angle1);
            const y1 = centerY + radius * Math.sin(angle1);
            const x2 = centerX + radius * Math.cos(angle2);
            const y2 = centerY + radius * Math.sin(angle2);
            
            const line = document.createElementNS('http://www.w3.org/2000/svg', 'line');
            line.setAttribute('x1', x1);
            line.setAttribute('y1', y1);
            line.setAttribute('x2', x2);
            line.setAttribute('y2', y2);
            line.setAttribute('stroke', 'rgba(130, 164, 115, 0.2)');
            line.setAttribute('stroke-width', '1');
            g.appendChild(line);
        }
    }
    
    // Draw nodes
    ais.forEach((ai, index) => {
        const angle = (index / ais.length) * 2 * Math.PI - Math.PI / 2;
        const x = centerX + radius * Math.cos(angle);
        const y = centerY + radius * Math.sin(angle);
        
        // Node group
        const nodeGroup = document.createElementNS('http://www.w3.org/2000/svg', 'g');
        nodeGroup.setAttribute('class', 'ai-node');
        
        // Determine color based on status
        let color = '#878787'; // not active
        let glowColor = 'rgba(135, 135, 135, 0.3)';
        
        if (ai.status === 'active') {
            color = '#82A473';
            glowColor = 'rgba(130, 164, 115, 0.6)';
        } else if (ai.status === 'idle') {
            color = '#f59e0b';
            glowColor = 'rgba(245, 158, 11, 0.4)';
        }
        
        // Glow effect
        const glow = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
        glow.setAttribute('cx', x);
        glow.setAttribute('cy', y);
        glow.setAttribute('r', '22');
        glow.setAttribute('fill', glowColor);
        glow.setAttribute('filter', 'blur(10px)');
        nodeGroup.appendChild(glow);
        
        // Main circle
        const circle = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
        circle.setAttribute('cx', x);
        circle.setAttribute('cy', y);
        circle.setAttribute('r', '18');
        circle.setAttribute('fill', 'rgba(10, 10, 10, 0.9)');
        circle.setAttribute('stroke', color);
        circle.setAttribute('stroke-width', '2');
        nodeGroup.appendChild(circle);
        
        // AI name
        const name = document.createElementNS('http://www.w3.org/2000/svg', 'text');
        name.setAttribute('x', x);
        name.setAttribute('y', y - 28);
        name.setAttribute('text-anchor', 'middle');
        name.setAttribute('fill', color);
        name.setAttribute('font-family', 'JetBrains Mono');
        name.setAttribute('font-size', '12');
        name.setAttribute('font-weight', '700');
        name.textContent = ai.name;
        nodeGroup.appendChild(name);
        
        // Last command (if available)
        if (ai.lastCommand) {
            const cmd = document.createElementNS('http://www.w3.org/2000/svg', 'text');
            cmd.setAttribute('x', x);
            cmd.setAttribute('y', y + 35);
            cmd.setAttribute('text-anchor', 'middle');
            cmd.setAttribute('fill', 'rgba(255, 255, 255, 0.5)');
            cmd.setAttribute('font-family', 'JetBrains Mono');
            cmd.setAttribute('font-size', '9');
            // Truncate long commands
            const truncated = ai.lastCommand.length > 25 ? 
                ai.lastCommand.substring(0, 25) + '...' : ai.lastCommand;
            cmd.textContent = truncated;
            nodeGroup.appendChild(cmd);
        }
        
        // Data flow indicator (animated)
        if (ai.status === 'active') {
            const flow = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
            flow.setAttribute('cx', x);
            flow.setAttribute('cy', y);
            flow.setAttribute('r', '8');
            flow.setAttribute('fill', 'none');
            flow.setAttribute('stroke', color);
            flow.setAttribute('stroke-width', '1');
            flow.setAttribute('opacity', '0.8');
            
            // Animate the flow
            const animate = document.createElementNS('http://www.w3.org/2000/svg', 'animate');
            animate.setAttribute('attributeName', 'r');
            animate.setAttribute('from', '8');
            animate.setAttribute('to', '25');
            animate.setAttribute('dur', '2s');
            animate.setAttribute('repeatCount', 'indefinite');
            flow.appendChild(animate);
            
            const animateOpacity = document.createElementNS('http://www.w3.org/2000/svg', 'animate');
            animateOpacity.setAttribute('attributeName', 'opacity');
            animateOpacity.setAttribute('from', '0.8');
            animateOpacity.setAttribute('to', '0');
            animateOpacity.setAttribute('dur', '2s');
            animateOpacity.setAttribute('repeatCount', 'indefinite');
            flow.appendChild(animateOpacity);
            
            nodeGroup.appendChild(flow);
        }
        
        svg.appendChild(nodeGroup);
    });
}

// Check all backends
async function checkAllBackends() {
    updateOverallStatus('checking');

    try {
        const response = await fetch(`${API_BASE}/health`);
        const data = await response.json();

        healthData = data;

        updatePostgreSQLStatus(data.postgresql);
        updateRedisStatus(data.redis);
        updateDuckDBStatus(data.duckdb);
        updateFallbackChain(data.activeBackend);
        updateStats(data.stats);
        updateOverallStatus(data.overall);

        // Update AI nodes if available
        if (data.activeAIs) {
            updateAINodeGraph(data.activeAIs);
        }

    } catch (error) {
        console.error('Health check failed:', error);
        updateOverallStatus('error');
        showOfflineMode();
    }
}

// Update PostgreSQL status
function updatePostgreSQLStatus(data) {
    const card = document.getElementById('postgresql-card');
    const connection = document.getElementById('pg-connection');
    const latency = document.getElementById('pg-latency');
    const notes = document.getElementById('pg-notes');
    const pool = document.getElementById('pg-pool');
    const bar = document.getElementById('pg-bar');

    if (data.connected) {
        card.classList.add('active');
        card.classList.remove('error', 'warning');
        connection.textContent = 'CONNECTED';
        connection.classList.add('success');
        latency.textContent = `${data.latency}ms`;
        notes.textContent = data.noteCount.toLocaleString();
        pool.textContent = `${data.poolUsed}/${data.poolMax}`;
        bar.style.width = '100%';
    } else {
        card.classList.add('error');
        card.classList.remove('active', 'warning');
        connection.textContent = data.error || 'UNAVAILABLE';
        connection.classList.add('error');
        latency.textContent = '--';
        notes.textContent = '--';
        pool.textContent = '--';
        bar.style.width = '0%';
    }
}

// Update Redis status
function updateRedisStatus(data) {
    const card = document.getElementById('redis-card');
    const connection = document.getElementById('redis-connection');
    const latency = document.getElementById('redis-latency');
    const pubsub = document.getElementById('redis-pubsub');
    const memory = document.getElementById('redis-memory');
    const bar = document.getElementById('redis-bar');

    if (data.connected) {
        card.classList.add('active');
        card.classList.remove('error', 'warning');
        connection.textContent = 'CONNECTED';
        connection.classList.add('success');
        latency.textContent = `${data.latency}ms`;
        pubsub.textContent = data.pubsubActive ? 'ACTIVE' : 'INACTIVE';
        memory.textContent = data.memoryUsed || '--';
        bar.style.width = '100%';
    } else {
        card.classList.add('error');
        card.classList.remove('active', 'warning');
        connection.textContent = data.error || 'UNAVAILABLE';
        connection.classList.add('error');
        latency.textContent = '--';
        pubsub.textContent = '--';
        memory.textContent = '--';
        bar.style.width = '0%';
    }
}

// Update DuckDB status
function updateDuckDBStatus(data) {
    const card = document.getElementById('duckdb-card');
    const connection = document.getElementById('duckdb-connection');
    const size = document.getElementById('duckdb-size');
    const notes = document.getElementById('duckdb-notes');
    const bar = document.getElementById('duckdb-bar');

    // DuckDB is always available
    card.classList.add('active');
    card.classList.remove('error', 'warning');
    connection.textContent = 'ALWAYS AVAILABLE';
    connection.classList.add('success');
    size.textContent = data.sizeFormatted || '--';
    notes.textContent = data.noteCount.toLocaleString();
    bar.style.width = '100%';
}

// Update fallback chain visualization
function updateFallbackChain(activeBackend) {
    const backends = ['postgresql', 'redis', 'duckdb'];

    backends.forEach(backend => {
        const node = document.getElementById(`fallback-${backend}`);
        const status = document.getElementById(`fallback-${backend}-status`);

        if (backend === activeBackend) {
            node.classList.add('active');
            node.classList.remove('inactive');
            status.textContent = 'ACTIVE';
        } else if (healthData[backend] && healthData[backend].connected) {
            node.classList.remove('active', 'inactive');
            status.textContent = 'STANDBY';
        } else {
            node.classList.add('inactive');
            node.classList.remove('active');
            status.textContent = 'UNAVAILABLE';
        }
    });

    const activeBackendDisplay = document.getElementById('active-backend');
    activeBackendDisplay.textContent = activeBackend.toUpperCase();
}

// Update performance stats
function updateStats(stats) {
    document.getElementById('stat-writes').textContent = stats.writesPerSec || '--';
    document.getElementById('stat-reads').textContent = stats.readsPerSec || '--';
    document.getElementById('stat-latency').textContent = stats.avgLatency || '--';
    document.getElementById('stat-uptime').textContent = stats.uptime || '--';

    // Update stat bars
    const writesPercent = Math.min((stats.writesPerSec / 5000) * 100, 100);
    const readsPercent = Math.min((stats.readsPerSec / 5000) * 100, 100);
    const latencyPercent = Math.max(100 - ((stats.avgLatency / 100) * 100), 0);
    const uptimePercent = 100;

    document.getElementById('stat-writes-bar').style.width = `${writesPercent}%`;
    document.getElementById('stat-reads-bar').style.width = `${readsPercent}%`;
    document.getElementById('stat-latency-bar').style.width = `${latencyPercent}%`;
    document.getElementById('stat-uptime-bar').style.width = `${uptimePercent}%`;
}

// Update overall status badge
function updateOverallStatus(status) {
    const badge = document.getElementById('overall-status');
    const text = badge.querySelector('.status-text');

    badge.classList.remove('active', 'warning', 'error');

    if (status === 'checking') {
        text.textContent = 'CHECKING...';
    } else if (status === 'healthy') {
        badge.classList.add('active');
        text.textContent = 'ALL SYSTEMS OPERATIONAL';
    } else if (status === 'degraded') {
        badge.classList.add('warning');
        text.textContent = 'RUNNING ON FALLBACK';
    } else if (status === 'error') {
        badge.classList.add('error');
        text.textContent = 'SYSTEM ERROR';
    }
}

// Show offline mode
function showOfflineMode() {
    updatePostgreSQLStatus({connected: false, error: 'API UNREACHABLE'});
    updateRedisStatus({connected: false, error: 'API UNREACHABLE'});
    updateDuckDBStatus({connected: true, noteCount: 0, sizeFormatted: 'Unknown'});
    updateFallbackChain('duckdb');
}

// Test individual backend
async function testBackend(backend) {
    const button = event.target;
    const originalText = button.textContent;
    button.textContent = 'TESTING...';
    button.disabled = true;

    try {
        const response = await fetch(`${API_BASE}/test/${backend}`);
        const data = await response.json();

        if (data.success) {
            alert(`✓ ${backend.toUpperCase()} connection successful!\n\nLatency: ${data.latency}ms\nNotes: ${data.noteCount}`);
        } else {
            alert(`✗ ${backend.toUpperCase()} connection failed!\n\nError: ${data.error}`);
        }

        await checkAllBackends();

    } catch (error) {
        alert(`✗ Test failed!\n\nError: ${error.message}`);
    } finally {
        button.textContent = originalText;
        button.disabled = false;
    }
}

// Configure backend (modal dialogs omitted for brevity - same as original)
function configureBackend(backend) {
    // Same implementation as original...
    alert(`Configuration for ${backend} - implement full modal UI`);
}

function closeModal() {
    const modal = document.getElementById('config-modal');
    modal.style.display = 'none';
}

async function refreshAll() {
    await checkAllBackends();
}

function startAutoRefresh() {
    setInterval(checkAllBackends, 10000);
}

// Recreate data stream on window resize
let resizeTimeout;
window.addEventListener('resize', () => {
    clearTimeout(resizeTimeout);
    resizeTimeout = setTimeout(() => {
        const dataStream = document.getElementById('dataStream');
        dataStream.innerHTML = '';
        initDataStream();
    }, 500);
});

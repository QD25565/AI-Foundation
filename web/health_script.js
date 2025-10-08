// Teambook Health Monitor - JavaScript

// Real code snippets for falling background (matching website aesthetic)
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
    "logger.info('Notebook v1.0.0 initialized');",
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

// Falling code strings background (matching website)
function initFallingCode() {
    const dataStream = document.getElementById('dataStream');
    const columns = Math.floor(window.innerWidth / 80) * 3;  // 3x denser

    for (let i = 0; i < columns; i++) {
        createCodeColumn(i);
    }

    function createCodeColumn(index) {
        const column = document.createElement('div');
        column.className = 'data-column';

        // Random horizontal position
        column.style.left = (Math.random() * window.innerWidth) + 'px';

        // Random code snippet
        const codeSnippet = codeSnippets[Math.floor(Math.random() * codeSnippets.length)];
        column.textContent = codeSnippet;

        // Random animation duration (12-25 seconds)
        const duration = (Math.random() * 13 + 12) + 's';
        column.style.animationDuration = duration;

        // Random delay for staggered start
        const delay = (Math.random() * 10) + 's';
        column.style.animationDelay = delay;

        dataStream.appendChild(column);

        // Recreate after animation completes
        const totalDuration = (parseFloat(duration) + parseFloat(delay)) * 1000;
        setTimeout(() => {
            column.remove();
            createCodeColumn(index);
        }, totalDuration);
    }
}

// Recreate columns on window resize (debounced)
let resizeTimeout;
window.addEventListener('resize', () => {
    clearTimeout(resizeTimeout);
    resizeTimeout = setTimeout(() => {
        const dataStream = document.getElementById('dataStream');
        dataStream.innerHTML = '';
        initFallingCode();
    }, 500);
});

// API endpoint (will be served by Python backend)
const API_BASE = 'http://localhost:8765/api';

// State
let healthData = {
    postgresql: null,
    redis: null,
    duckdb: null,
    activeBackend: 'unknown',
    stats: {}
};

// Initialize on page load
document.addEventListener('DOMContentLoaded', () => {
    initFallingCode();
    initAINetwork();
    checkAllBackends();
    startAutoRefresh();
});

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

    // Update stat bars (max at 5000 ops/sec for writes, 10ms for latency)
    const writesPercent = Math.min((stats.writesPerSec / 5000) * 100, 100);
    const readsPercent = Math.min((stats.readsPerSec / 5000) * 100, 100);
    const latencyPercent = Math.max(100 - ((stats.avgLatency / 100) * 100), 0);
    const uptimePercent = 100; // Always full if system is up

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

// Show offline mode when API is unreachable
function showOfflineMode() {
    // Default to DuckDB when offline
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

        // Refresh status
        await checkAllBackends();

    } catch (error) {
        alert(`✗ Test failed!\n\nError: ${error.message}`);
    } finally {
        button.textContent = originalText;
        button.disabled = false;
    }
}

// Configure backend
function configureBackend(backend) {
    const modal = document.getElementById('config-modal');
    const title = document.getElementById('modal-title');
    const body = document.getElementById('modal-body');

    title.textContent = `CONFIGURE ${backend.toUpperCase()}`;

    let content = '';

    if (backend === 'postgresql') {
        content = `
            <div class="config-form">
                <h3 style="color: var(--neon-green); margin-bottom: 20px;">PostgreSQL Configuration</h3>

                <div class="form-group">
                    <label>Connection URL:</label>
                    <input type="text" id="postgres-url" placeholder="postgresql://user:pass@host:5432/db"
                           value="${healthData.postgresql?.url || ''}">
                </div>

                <div class="form-group">
                    <label>Or configure manually:</label>
                    <input type="text" id="pg-host" placeholder="Host (localhost)">
                    <input type="text" id="pg-port" placeholder="Port (5432)">
                    <input type="text" id="pg-user" placeholder="Username">
                    <input type="password" id="pg-pass" placeholder="Password">
                    <input type="text" id="pg-db" placeholder="Database (teambook)">
                </div>

                <div class="form-group">
                    <label>Pool Settings:</label>
                    <input type="number" id="pg-min-conn" placeholder="Min Connections (2)" value="2">
                    <input type="number" id="pg-max-conn" placeholder="Max Connections (10)" value="10">
                </div>

                <button onclick="savePostgreSQLConfig()" style="width: 100%; margin-top: 20px;">SAVE & TEST</button>
            </div>
        `;
    } else if (backend === 'redis') {
        content = `
            <div class="config-form">
                <h3 style="color: var(--neon-green); margin-bottom: 20px;">Redis Configuration</h3>

                <div class="form-group">
                    <label>Redis URL:</label>
                    <input type="text" id="redis-url" placeholder="redis://localhost:6379/0"
                           value="${healthData.redis?.url || ''}">
                </div>

                <div class="form-group">
                    <label>Or configure manually:</label>
                    <input type="text" id="redis-host" placeholder="Host (localhost)">
                    <input type="text" id="redis-port" placeholder="Port (6379)">
                    <input type="password" id="redis-password" placeholder="Password (optional)">
                    <input type="number" id="redis-db" placeholder="Database (0)" value="0">
                </div>

                <div class="form-group">
                    <label style="display: flex; align-items: center; gap: 10px;">
                        <input type="checkbox" id="redis-pubsub" checked>
                        Enable Pub/Sub (real-time notifications)
                    </label>
                </div>

                <button onclick="saveRedisConfig()" style="width: 100%; margin-top: 20px;">SAVE & TEST</button>
            </div>
        `;
    } else if (backend === 'duckdb') {
        content = `
            <div class="config-form">
                <h3 style="color: var(--neon-green); margin-bottom: 20px;">DuckDB Configuration</h3>

                <p style="margin-bottom: 20px; color: var(--battleship);">
                    DuckDB is zero-configuration and always available as a fallback.
                </p>

                <div class="form-group">
                    <label>Database Path:</label>
                    <input type="text" id="duckdb-path" placeholder="Leave empty for default location"
                           value="${healthData.duckdb?.path || ''}">
                </div>

                <div class="form-group">
                    <label style="display: flex; align-items: center; gap: 10px;">
                        <input type="checkbox" id="duckdb-readonly" checked>
                        Read-only mode (safer for concurrent access)
                    </label>
                </div>

                <button onclick="saveDuckDBConfig()" style="width: 100%; margin-top: 20px;">SAVE</button>
            </div>
        `;
    }

    body.innerHTML = content;
    modal.style.display = 'flex';
}

// Save PostgreSQL config
async function savePostgreSQLConfig() {
    const url = document.getElementById('postgres-url').value;

    // Build URL from manual fields if URL field is empty
    let connectionUrl = url;
    if (!url) {
        const host = document.getElementById('pg-host').value || 'localhost';
        const port = document.getElementById('pg-port').value || '5432';
        const user = document.getElementById('pg-user').value;
        const pass = document.getElementById('pg-pass').value;
        const db = document.getElementById('pg-db').value || 'teambook';

        if (!user) {
            alert('Please provide a username');
            return;
        }

        connectionUrl = `postgresql://${user}:${pass}@${host}:${port}/${db}`;
    }

    const minConn = document.getElementById('pg-min-conn').value;
    const maxConn = document.getElementById('pg-max-conn').value;

    try {
        const response = await fetch(`${API_BASE}/config/postgresql`, {
            method: 'POST',
            headers: {'Content-Type': 'application/json'},
            body: JSON.stringify({
                url: connectionUrl,
                minConn: parseInt(minConn),
                maxConn: parseInt(maxConn)
            })
        });

        const data = await response.json();

        if (data.success) {
            alert('✓ PostgreSQL configured successfully!');
            closeModal();
            await checkAllBackends();
        } else {
            alert(`✗ Configuration failed!\n\nError: ${data.error}`);
        }
    } catch (error) {
        alert(`✗ Configuration failed!\n\nError: ${error.message}`);
    }
}

// Save Redis config
async function saveRedisConfig() {
    const url = document.getElementById('redis-url').value;

    let connectionUrl = url;
    if (!url) {
        const host = document.getElementById('redis-host').value || 'localhost';
        const port = document.getElementById('redis-port').value || '6379';
        const password = document.getElementById('redis-password').value;
        const db = document.getElementById('redis-db').value || '0';

        if (password) {
            connectionUrl = `redis://:${password}@${host}:${port}/${db}`;
        } else {
            connectionUrl = `redis://${host}:${port}/${db}`;
        }
    }

    const pubsub = document.getElementById('redis-pubsub').checked;

    try {
        const response = await fetch(`${API_BASE}/config/redis`, {
            method: 'POST',
            headers: {'Content-Type': 'application/json'},
            body: JSON.stringify({
                url: connectionUrl,
                pubsub: pubsub
            })
        });

        const data = await response.json();

        if (data.success) {
            alert('✓ Redis configured successfully!');
            closeModal();
            await checkAllBackends();
        } else {
            alert(`✗ Configuration failed!\n\nError: ${data.error}`);
        }
    } catch (error) {
        alert(`✗ Configuration failed!\n\nError: ${error.message}`);
    }
}

// Save DuckDB config
async function saveDuckDBConfig() {
    const path = document.getElementById('duckdb-path').value;
    const readonly = document.getElementById('duckdb-readonly').checked;

    try {
        const response = await fetch(`${API_BASE}/config/duckdb`, {
            method: 'POST',
            headers: {'Content-Type': 'application/json'},
            body: JSON.stringify({
                path: path,
                readonly: readonly
            })
        });

        const data = await response.json();

        if (data.success) {
            alert('✓ DuckDB configured successfully!');
            closeModal();
            await checkAllBackends();
        } else {
            alert(`✗ Configuration failed!\n\nError: ${data.error}`);
        }
    } catch (error) {
        alert(`✗ Configuration failed!\n\nError: ${error.message}`);
    }
}

// Close modal
function closeModal() {
    const modal = document.getElementById('config-modal');
    modal.style.display = 'none';
}

// Refresh all
async function refreshAll() {
    await checkAllBackends();
}

// Auto-refresh every 10 seconds
function startAutoRefresh() {
    setInterval(checkAllBackends, 10000);
}

// Add CSS for form elements
const formStyles = document.createElement('style');
formStyles.textContent = `
    .config-form {
        color: var(--battleship);
    }

    .form-group {
        margin-bottom: 25px;
    }

    .form-group label {
        display: block;
        margin-bottom: 10px;
        font-size: 0.9rem;
        color: var(--neon-green);
        letter-spacing: 0.1em;
        font-weight: 500;
    }

    .form-group input[type="text"],
    .form-group input[type="password"],
    .form-group input[type="number"] {
        width: 100%;
        padding: 12px;
        background: rgba(23, 23, 23, 0.8);
        border: 1px solid var(--border);
        color: var(--neon-green);
        font-family: 'JetBrains Mono', monospace;
        font-size: 0.9rem;
        margin-bottom: 10px;
        transition: all 0.3s ease;
    }

    .form-group input[type="text"]:focus,
    .form-group input[type="password"]:focus,
    .form-group input[type="number"]:focus {
        outline: none;
        border-color: var(--neon-green);
        box-shadow: 0 0 10px rgba(130, 164, 115, 0.3);
    }

    .form-group input[type="checkbox"] {
        width: 18px;
        height: 18px;
        accent-color: var(--neon-green);
    }
`;
document.head.appendChild(formStyles);

// ====================================================================
// AI NETWORK VISUALIZATION - CYBERPUNK CYBERSPACE
// ====================================================================

let aiNetworkCanvas, aiNetworkCtx;
let aiNodes = [];
let connections = [];
let animationFrameId;
let messageParticles = []; // Store active message particles

// Initialize AI Network canvas
function initAINetwork() {
    aiNetworkCanvas = document.getElementById('ai-network-canvas');
    if (!aiNetworkCanvas) return;

    aiNetworkCtx = aiNetworkCanvas.getContext('2d');
    resizeAINetworkCanvas();

    window.addEventListener('resize', resizeAINetworkCanvas);

    // Start animation loop
    animateAINetwork();

    // Fetch AI data and update every 5 seconds
    updateAINodes();
    setInterval(updateAINodes, 5000);
}

function resizeAINetworkCanvas() {
    if (!aiNetworkCanvas) return;
    aiNetworkCanvas.width = aiNetworkCanvas.offsetWidth;
    aiNetworkCanvas.height = aiNetworkCanvas.offsetHeight;
}

// Fetch AI node data from API
async function updateAINodes() {
    try {
        const response = await fetch(`${API_BASE}/ai-network`);
        const data = await response.json();

        // Handle error response from backend
        if (data.error) {
            console.error('Backend error:', data.error);
            showAINetworkError(data.error);
            return;
        }

        const oldNodes = aiNodes;
        aiNodes = data.nodes || [];
        connections = data.connections || [];

        // Create message particles for new activity (only on real data updates)
        if (oldNodes.length > 0) {
            createParticlesFromActivity(oldNodes, aiNodes);
        }

        // Update AI list cards
        updateAIList(aiNodes);

    } catch (error) {
        console.error('Failed to fetch AI network:', error);
        showAINetworkError(error.message);
    }
}

// Create particles based on REAL message activity changes
function createParticlesFromActivity(oldNodes, newNodes) {
    newNodes.forEach(newNode => {
        const oldNode = oldNodes.find(n => n.id === newNode.id);
        if (!oldNode) return;

        // Check if sent count increased (real new messages)
        const sentDiff = (newNode.sent || 0) - (oldNode.sent || 0);
        if (sentDiff > 0) {
            // Create particles for each new message sent
            for (let i = 0; i < Math.min(sentDiff, 5); i++) { // Cap at 5 particles per update
                // Find connections from this node
                connections.forEach(conn => {
                    if (conn.from === newNode.id) {
                        messageParticles.push({
                            from: conn.from,
                            to: conn.to,
                            progress: 0,
                            speed: 0.015 + Math.random() * 0.01, // Vary speed slightly
                            createdAt: Date.now() + (i * 100) // Stagger particles
                        });
                    }
                });
            }
        }
    });
}

// Show error message when AI network data cannot be loaded
function showAINetworkError(errorMessage) {
    aiNodes = [];
    connections = [];

    const aiList = document.getElementById('ai-list');
    if (!aiList) return;

    aiList.innerHTML = `
        <div class="ai-error-card">
            <div class="error-icon">⚠</div>
            <div class="error-title">AI NETWORK UNAVAILABLE</div>
            <div class="error-message">${errorMessage}</div>
            <div class="error-help">
                Check that:
                <ul>
                    <li>Health server is running (http://localhost:8765)</li>
                    <li>Teambook storage is accessible</li>
                    <li>Backend database is configured</li>
                </ul>
            </div>
        </div>
    `;
}

// Animate AI network
function animateAINetwork() {
    if (!aiNetworkCtx || !aiNetworkCanvas) return;

    const width = aiNetworkCanvas.width;
    const height = aiNetworkCanvas.height;

    // Clear canvas with fade effect
    aiNetworkCtx.fillStyle = 'rgba(10, 10, 10, 0.1)';
    aiNetworkCtx.fillRect(0, 0, width, height);

    // Draw connections (static lines based on REAL connection strength)
    connections.forEach(conn => {
        const fromNode = aiNodes.find(n => n.id === conn.from);
        const toNode = aiNodes.find(n => n.id === conn.to);

        if (fromNode && toNode) {
            const fromX = fromNode.x * width;
            const fromY = fromNode.y * height;
            const toX = toNode.x * width;
            const toY = toNode.y * height;

            // Draw connection line (opacity based on REAL strength)
            aiNetworkCtx.strokeStyle = `rgba(130, 164, 115, ${conn.strength * 0.3})`;
            aiNetworkCtx.lineWidth = 1 + (conn.strength * 2);
            aiNetworkCtx.beginPath();
            aiNetworkCtx.moveTo(fromX, fromY);
            aiNetworkCtx.lineTo(toX, toY);
            aiNetworkCtx.stroke();
        }
    });

    // Update and draw REAL message particles (only when actual messages sent)
    const now = Date.now();
    messageParticles = messageParticles.filter(particle => {
        // Only show particle if it's time (staggered start)
        if (now < particle.createdAt) return true;

        const fromNode = aiNodes.find(n => n.id === particle.from);
        const toNode = aiNodes.find(n => n.id === particle.to);

        if (!fromNode || !toNode) return false;

        // Update progress
        particle.progress += particle.speed;

        // Remove if reached destination
        if (particle.progress >= 1) return false;

        // Draw particle
        const fromX = fromNode.x * width;
        const fromY = fromNode.y * height;
        const toX = toNode.x * width;
        const toY = toNode.y * height;

        const particleX = fromX + (toX - fromX) * particle.progress;
        const particleY = fromY + (toY - fromY) * particle.progress;

        // Particle with trail effect
        const alpha = 1 - particle.progress; // Fade as it travels
        aiNetworkCtx.fillStyle = `rgba(130, 164, 115, ${alpha})`;
        aiNetworkCtx.beginPath();
        aiNetworkCtx.arc(particleX, particleY, 3, 0, Math.PI * 2);
        aiNetworkCtx.fill();

        // Glow effect
        const gradient = aiNetworkCtx.createRadialGradient(particleX, particleY, 0, particleX, particleY, 8);
        gradient.addColorStop(0, `rgba(130, 164, 115, ${alpha * 0.6})`);
        gradient.addColorStop(1, 'transparent');
        aiNetworkCtx.fillStyle = gradient;
        aiNetworkCtx.fillRect(particleX - 8, particleY - 8, 16, 16);

        return true; // Keep particle
    });

    // Draw nodes
    aiNodes.forEach(node => {
        const x = node.x * width;
        const y = node.y * height;

        // Determine node color based on status
        let color, glowColor;
        if (node.status === 'active') {
            color = '#82A473';
            glowColor = 'rgba(130, 164, 115, 0.6)';
        } else if (node.status === 'idle') {
            color = '#f59e0b';
            glowColor = 'rgba(245, 158, 11, 0.5)';
        } else {
            color = '#878787';
            glowColor = 'rgba(135, 135, 135, 0.3)';
        }

        // Draw glow
        const gradient = aiNetworkCtx.createRadialGradient(x, y, 0, x, y, 30);
        gradient.addColorStop(0, glowColor);
        gradient.addColorStop(1, 'transparent');
        aiNetworkCtx.fillStyle = gradient;
        aiNetworkCtx.fillRect(x - 30, y - 30, 60, 60);

        // Draw outer ring (pulsing for active)
        if (node.status === 'active') {
            const pulse = Math.sin(Date.now() / 500) * 0.3 + 0.7;
            aiNetworkCtx.strokeStyle = `rgba(130, 164, 115, ${pulse})`;
            aiNetworkCtx.lineWidth = 2;
            aiNetworkCtx.beginPath();
            aiNetworkCtx.arc(x, y, 18, 0, Math.PI * 2);
            aiNetworkCtx.stroke();
        }

        // Draw node circle
        aiNetworkCtx.fillStyle = color;
        aiNetworkCtx.beginPath();
        aiNetworkCtx.arc(x, y, 12, 0, Math.PI * 2);
        aiNetworkCtx.fill();

        // Draw border
        aiNetworkCtx.strokeStyle = 'rgba(10, 10, 10, 0.8)';
        aiNetworkCtx.lineWidth = 2;
        aiNetworkCtx.stroke();

        // Draw name label
        aiNetworkCtx.fillStyle = '#ffffff';
        aiNetworkCtx.font = '11px JetBrains Mono';
        aiNetworkCtx.textAlign = 'center';
        aiNetworkCtx.fillText(node.name, x, y - 25);
    });

    animationFrameId = requestAnimationFrame(animateAINetwork);
}

// Update AI list (cards below canvas)
function updateAIList(nodes) {
    const aiList = document.getElementById('ai-list');
    if (!aiList) return;

    aiList.innerHTML = nodes.map(node => `
        <div class="ai-node-card ${node.status}">
            <div class="ai-node-header">
                <div class="ai-node-name">${node.name}</div>
                <div class="ai-status-indicator ${node.status}">
                    <div class="ai-status-dot"></div>
                    <span class="ai-status-text">${node.status.toUpperCase()}</span>
                </div>
            </div>
            <div class="ai-node-body">
                <div class="ai-info-row">
                    <span class="ai-info-label">ID:</span>
                    <span class="ai-info-value">${node.id}</span>
                </div>
                <div class="ai-info-row">
                    <span class="ai-info-label">STATUS:</span>
                    <span class="ai-info-value">${node.status.toUpperCase()}</span>
                </div>
            </div>
            ${node.lastCommand ? `
                <div class="ai-last-command">
                    LAST: ${node.lastCommand}
                </div>
            ` : ''}
            <div class="ai-flow-indicators">
                <div class="flow-indicator sent">
                    <div class="flow-count">${node.sent || 0}</div>
                    <div class="flow-label">SENT</div>
                </div>
                <div class="flow-indicator received">
                    <div class="flow-count">${node.received || 0}</div>
                    <div class="flow-label">RECEIVED</div>
                </div>
            </div>
        </div>
    `).join('');
}

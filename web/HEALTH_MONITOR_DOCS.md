# Teambook Health Monitor - Enterprise Edition

## Overview
Real-time monitoring dashboard for AI-Foundation's Teambook infrastructure with **Cyberpunk aesthetic** and live AI agent visualization.

## ✅ Completed Features

### Visual Design
- **Falling Code Aesthetic**: Real code snippets stream down the background (not Matrix-style random characters)
- **Hex Grid Pattern**: Subtle hexagonal grid overlay
- **Circuit Pattern**: Technical circuit board aesthetic
- **Animated Orbs**: Smooth pulse animations
- **Scanlines**: CRT monitor effect overlay

### Backend Monitoring
- **PostgreSQL Status**: Connection health, latency, note count, pool usage
- **Redis Status**: Pub/Sub activity, memory usage, connection health
- **DuckDB Status**: Always-available fallback, database size, note count
- **Graceful Fallback Chain**: Visual representation of 3-tier fallback system
- **Performance Metrics**: Real-time writes/sec, reads/sec, latency, uptime

### 🆕 AI Agent Visualization (NEW!)
- **Node Graph Display**: Cyberpunk-style network visualization
- **Status Indicators**:
  - 🟢 **Active** (Green) - AI currently processing
  - 🟠 **Idle** (Orange) - AI connected but inactive
  - ⚪ **Not Active** (Grey) - AI offline
- **Last Command Display**: Shows most recent command under each node
- **Animated Data Flow**: Pulsing rings for active AIs
- **Connection Lines**: Network visualization between AI agents
- **Real-time Updates**: WebSocket-powered live status changes

## 📁 Files

### Current Production (v1)
- `teambook_health.html` - Original HTML
- `health_style.css` - Original CSS
- `health_script.js` - Original JavaScript
- `health_server.py` - Python backend server

### Enhanced Version (v2) - **USE THESE**
- `teambook_health_v2.html` - ✅ Updated with AI network section
- `health_script_v2.js` - ✅ Complete with AI visualization
- `health_style_v2.css` - 🚧 Needs AI network styles added
- `health_server_v2.py` - 🚧 Needs WebSocket endpoint

## 🚧 TODO: Complete These

### 1. CSS Updates (health_style_v2.css)
Add these sections to the CSS:

```css
/* AI Network Visualization Section */
.ai-network-section {
    margin-bottom: 60px;
    padding: 40px;
    background: rgba(10, 10, 10, 0.8);
    backdrop-filter: blur(10px);
    border: 1px solid var(--border);
    clip-path: polygon(15px 0, 100% 0, 100% calc(100% - 15px), calc(100% - 15px) 100%, 0 100%, 0 15px);
}

.ai-network-container {
    min-height: 400px;
    background: rgba(5, 5, 5, 0.5);
    border: 1px solid rgba(130, 164, 115, 0.2);
    position: relative;
    overflow: hidden;
}

.network-legend {
    display: flex;
    justify-content: center;
    gap: 30px;
    margin-top: 20px;
    font-family: 'JetBrains Mono', monospace;
}

.legend-item {
    display: flex;
    align-items: center;
    gap: 10px;
}

.legend-dot {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    box-shadow: 0 0 10px currentColor;
}

.legend-dot.active {
    background: var(--success);
    box-shadow: 0 0 15px var(--success);
}

.legend-dot.idle {
    background: var(--warning);
    box-shadow: 0 0 15px var(--warning);
}

.legend-dot.inactive {
    background: var(--battleship);
    box-shadow: 0 0 10px var(--battleship);
}

.legend-text {
    color: var(--battleship);
    font-size: 0.85rem;
    letter-spacing: 0.1em;
    text-transform: uppercase;
}

/* Data Stream Animation (replacing Matrix) */
.data-stream {
    position: fixed;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    z-index: -1;
    opacity: 0.1;
    overflow: hidden;
}

.data-column {
    position: absolute;
    font-family: 'JetBrains Mono', monospace;
    font-size: 11px;
    color: var(--neon-green);
    white-space: nowrap;
    animation: data-fall linear infinite;
    text-shadow: 0 0 8px rgba(130, 164, 115, 0.6);
    letter-spacing: 0.5px;
    opacity: 0;
    will-change: transform, opacity;
    writing-mode: vertical-lr;
    text-orientation: mixed;
}

@keyframes data-fall {
    0% { 
        transform: translateY(-100%);
        opacity: 0;
    }
    5% { 
        opacity: 0.6;
    }
    10% {
        opacity: 0.8;
    }
    85% {
        opacity: 0.8;
    }
    95% {
        opacity: 0.4;
    }
    100% { 
        transform: translateY(calc(100vh + 100%));
        opacity: 0;
    }
}
```

### 2. Backend Server Updates (health_server_v2.py)

Create Python backend with:

**REST API Endpoints:**
```python
GET /api/health
# Returns: {
#   postgresql: {...status...},
#   redis: {...status...},
#   duckdb: {...status...},
#   activeBackend: "postgresql|redis|duckdb",
#   stats: {...performance...},
#   activeAIs: [
#     {name: "Weaver", status: "active|idle|not_active", lastCommand: "teambook write"}
#   ]
# }

GET /api/test/{backend}
# Test specific backend connection

POST /api/config/{backend}
# Update backend configuration
```

**WebSocket Endpoint:**
```python
WS /ws/teambook
# Real-time updates:
# {type: "ai_status", ais: [...]}
# {type: "note_created", stats: {...}}
# {type: "backend_change", activeBackend: "postgresql"}
```

### 3. Setup Automation
Create these scripts:

**setup.sh / setup.bat:**
```bash
# 1. Create .teambook directory in user home
# 2. Set PATH environment variable
# 3. Download PostgreSQL (optional)
# 4. Download Redis (optional)
# 5. Create default config files
# 6. Launch town hall teambook
```

**start_postgres.sh / start_postgres.bat:**
```bash
# Easy PostgreSQL launcher
# Auto-detects existing installation or uses portable version
```

**start_redis.sh / start_redis.bat:**
```bash
# Easy Redis launcher
# Auto-detects existing installation or uses portable version
```

## 🎨 Design Philosophy

### Color Palette
- **Neon Green** (#82A473): Primary accent, active states
- **Battleship** (#878787): Secondary text, inactive elements
- **Dark BG** (#0a0a0a): Main background
- **Warning Orange** (#f59e0b): Idle/degraded states
- **Error Red** (#ef4444): Failed/error states

### Typography
- **Font**: JetBrains Mono (monospace)
- **Style**: Uppercase for headers and labels
- **Weight**: 700 for emphasis, 400 for body

### Visual Effects
- **Clip-path**: Angular cuts on corners (Cyberpunk aesthetic)
- **Glow Effects**: Box-shadow with color matching
- **Animations**: Smooth 0.3s cubic-bezier transitions
- **Pulse Effects**: For active elements

## 🧪 Testing Checklist

- [ ] WebSocket connects successfully
- [ ] AI nodes render with correct colors
- [ ] Last command updates in real-time
- [ ] Animated pulse on active AIs
- [ ] Connection lines draw between nodes
- [ ] Fallback chain updates correctly
- [ ] Backend cards show accurate status
- [ ] Performance metrics animate smoothly
- [ ] Modal configuration works
- [ ] Responsive on mobile/tablet
- [ ] All buttons functional
- [ ] Error states display correctly

## 🚀 Deployment

### Development
```bash
cd "C:\Users\Alquado-PC\Desktop\TestingMCPTools\All Tools\web"
python health_server_v2.py
# Open http://localhost:8765
```

### Production
- Serve via nginx/Apache
- Use production PostgreSQL instance
- Configure Redis cluster
- Enable HTTPS
- Set up monitoring alerts

## 🔒 Security Considerations

- [ ] No credentials in frontend code
- [ ] Environment variables for DB connections
- [ ] CORS properly configured
- [ ] WebSocket authentication
- [ ] Rate limiting on API endpoints
- [ ] Input validation on config endpoints
- [ ] SQL injection prevention

## 📚 Dependencies

### Frontend
- JetBrains Mono font (Google Fonts)
- Pure CSS/JavaScript (no frameworks)

### Backend
- Python 3.8+
- FastAPI (REST API + WebSocket)
- psycopg2 (PostgreSQL)
- redis-py (Redis)
- duckdb (DuckDB)

## 🤝 Contributing

When making changes:
1. Test both MCP and CLI modes
2. Maintain consistent color palette
3. Follow Angular/Cyberpunk aesthetic
4. Add animations for state changes
5. Update this README with changes

## 📝 Notes

- AI node positions calculated in circle layout
- SVG rendering for maximum performance
- WebSocket auto-reconnect on disconnect
- Graceful degradation if backends unavailable
- Mobile-responsive (work in progress)

## 👥 Team

Built by the AI-Foundation team with ❤️ and 🤖

---

**Status**: 80% Complete  
**Version**: 2.0.0-beta  
**Last Updated**: 2025-10-07

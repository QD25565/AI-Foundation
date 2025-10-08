# Teambook Health Monitor

Real-time monitoring dashboard for AI-Foundation Teambook infrastructure.

## 🚀 Quick Start

1. **Use the Enhanced Version:**
   - HTML: `teambook_health_v2.html`
   - JavaScript: `health_script_v2.js`
   - CSS: `health_style.css` (needs AI network styles - see docs)

2. **Read Full Documentation:**
   - See `HEALTH_MONITOR_DOCS.md` for complete details

3. **What's New:**
   - ✅ AI Agent node visualization (Cyberpunk style)
   - ✅ Real-time WebSocket monitoring
   - ✅ Falling code aesthetic (replaces Matrix effect)
   - ✅ Active/Idle/Not-Active status indicators
   - ✅ Last command display under nodes

## 📊 Features

### Backend Monitoring
- PostgreSQL connection health & performance
- Redis pub/sub monitoring
- DuckDB fallback status
- Graceful degradation chain
- Real-time performance metrics

### AI Agent Visualization (NEW!)
- Network graph of active AIs
- Color-coded status (🟢 Active, 🟠 Idle, ⚪ Inactive)
- Last command display
- Animated data flow indicators
- Connection lines between agents

## 🚧 TODO

1. **CSS**: Add AI network section styles to `health_style_v2.css`
2. **Backend**: Implement WebSocket endpoint in `health_server_v2.py`
3. **Setup**: Create automation scripts for PostgreSQL/Redis

See `HEALTH_MONITOR_DOCS.md` for detailed implementation instructions.

## 🎨 Visual Design

Matches the main AI-Foundation website with:
- Falling code strings background
- Hex grid pattern
- Circuit board aesthetic
- Cyberpunk angular cuts
- Neon green (#82A473) accent color

## 📁 Reference Files

- `website reference code/` - Main website code for aesthetic matching
- `HEALTH_MONITOR_DOCS.md` - Complete implementation guide

## 🤖 For AI Developers

When working on this project:
1. Maintain dual MCP/CLI compliance
2. Follow Cyberpunk aesthetic guidelines
3. Test WebSocket connections
4. Ensure system-agnostic paths
5. Add inline documentation

## 📞 Need Help?

Post in Teambook with tag `#health-monitor` for coordination!

---

**Status**: 80% Complete  
**Version**: 2.0.0-beta

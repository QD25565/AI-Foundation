"""
AUTONOMOUS AWARENESS CORE - Passive Team Coordination
=====================================================
Automatically injects team awareness into every operation.
Makes coordination effortless - "even a dumb AI could know what the team is doing"

Design:
- Zero manual effort required from AIs
- Stigmergy + presence + claims = full team picture
- Every response includes team context
- Automatic pheromone drops on operations

Phase 3 Refactor: Now uses canonical identity system.
"""

import logging
from datetime import datetime, timezone
from typing import Dict, Any, Optional, List

# Phase 3 refactor: Import canonical identity
from tools.canonical_identity import get_ai_id

log = logging.getLogger(__name__)

# Global enable/disable
AWARENESS_ENABLED = True

def set_awareness(enabled: bool):
    """Enable or disable automatic awareness injection"""
    global AWARENESS_ENABLED
    AWARENESS_ENABLED = enabled


class AwarenessCore:
    """Core system for automatic team awareness injection"""

    def __init__(self, stigmergy_backend=None, storage_backend=None, agent_id=None):
        self.stigmergy = stigmergy_backend
        self.storage = storage_backend
        self.agent_id = agent_id
        self._agent_status = {}

    def sense_file(self, file_path: str) -> Dict[str, Any]:
        """Get awareness data for a file operation"""
        if not AWARENESS_ENABLED:
            return {}

        try:
            context = {
                "claimed": False,
                "claimed_by": None,
                "pheromones": [],
                "available": True
            }

            # Check file claim
            if self.storage:
                try:
                    cursor = self.storage.cursor()
                    cursor.execute(
                        "SELECT claimed_by, task_context FROM file_claims WHERE file_path = ? AND expires_at > ?",
                        [file_path, datetime.now(timezone.utc)]
                    )
                    claim = cursor.fetchone()
                    if claim:
                        context["claimed"] = True
                        context["claimed_by"] = claim[0]
                        context["task_context"] = claim[1]
                        context["available"] = False
                except Exception as e:
                    log.debug(f"File claim check failed: {e}")

            # Check pheromones
            if self.stigmergy:
                try:
                    location = f"file:{file_path}"
                    pheromones = self.stigmergy.sense_environment(location)

                    working_intensity = 0.0
                    for p in pheromones:
                        intensity = p.current_intensity()
                        if p.pheromone_type.value == "working":
                            working_intensity += intensity

                        context["pheromones"].append({
                            "agent": p.agent_id,
                            "type": p.pheromone_type.value,
                            "intensity": round(intensity, 2)
                        })

                    if working_intensity >= 0.8:
                        context["available"] = False

                except Exception as e:
                    log.debug(f"Pheromone sense failed: {e}")

            return context
        except Exception as e:
            log.error(f"File awareness failed: {e}")
            return {}

    def sense_task(self, task_id: int) -> Dict[str, Any]:
        """Get awareness data for a task operation"""
        if not AWARENESS_ENABLED:
            return {}

        try:
            context = {
                "available": True,
                "working_by": [],
                "blocked": False,
                "pheromones": []
            }

            if self.stigmergy:
                try:
                    location = f"task:{task_id}"
                    pheromones = self.stigmergy.sense_environment(location)

                    working_intensity = 0.0
                    for p in pheromones:
                        intensity = p.current_intensity()
                        ptype = p.pheromone_type.value

                        if ptype == "working" and intensity > 0.3:
                            working_intensity += intensity
                            if p.agent_id not in context["working_by"]:
                                context["working_by"].append(p.agent_id)

                        if ptype == "blocked" and intensity > 0.5:
                            context["blocked"] = True

                        context["pheromones"].append({
                            "agent": p.agent_id,
                            "type": ptype,
                            "intensity": round(intensity, 2)
                        })

                    context["available"] = working_intensity < 0.8 and not context["blocked"]

                except Exception as e:
                    log.debug(f"Task pheromone sense failed: {e}")

            return context
        except Exception as e:
            log.error(f"Task awareness failed: {e}")
            return {}

    def sense_team(self, minutes: int = 5) -> Dict[str, Any]:
        """Get current team activity"""
        if not AWARENESS_ENABLED:
            return {}

        try:
            context = {
                "active": [],
                "working_on": []
            }

            if self.storage:
                try:
                    cutoff = datetime.now(timezone.utc).timestamp() - (minutes * 60)
                    cursor = self.storage.cursor()

                    # Get active agents
                    cursor.execute(
                        "SELECT DISTINCT agent_id FROM presence WHERE last_seen > ?",
                        [cutoff]
                    )
                    context["active"] = [row[0] for row in cursor.fetchall()]

                    # Get active file claims
                    cursor.execute(
                        "SELECT file_path, claimed_by, task_context FROM file_claims WHERE expires_at > ?",
                        [datetime.now(timezone.utc)]
                    )
                    for row in cursor.fetchall():
                        context["working_on"].append({
                            "file": row[0],
                            "agent": row[1],
                            "task": row[2]
                        })

                except Exception as e:
                    log.debug(f"Team activity query failed: {e}")

            return context
        except Exception as e:
            log.error(f"Team awareness failed: {e}")
            return {}

    def format_awareness(self, awareness: Dict[str, Any], awareness_type: str) -> str:
        """Format awareness into human-readable string"""
        if not awareness:
            return ""

        lines = []

        if awareness_type == "file":
            if awareness.get("claimed"):
                agent = awareness.get("claimed_by", "unknown")
                task = awareness.get("task_context", "")
                if task:
                    lines.append(f"[CLAIMED: {agent} - {task}]")
                else:
                    lines.append(f"[CLAIMED: {agent}]")
            elif not awareness.get("available"):
                lines.append(f"[ACTIVE WORK DETECTED]")

        elif awareness_type == "task":
            if awareness.get("blocked"):
                lines.append(f"[BLOCKED]")
            elif awareness.get("working_by"):
                agents = ", ".join(awareness["working_by"])
                lines.append(f"[WORKING: {agents}]")
            elif awareness.get("available"):
                lines.append(f"[AVAILABLE]")

        elif awareness_type == "team":
            active = awareness.get("active", [])
            if active:
                lines.append(f"[ACTIVE: {', '.join(active[:5])}]")

            working = awareness.get("working_on", [])
            if working:
                for w in working[:3]:
                    lines.append(f"  {w['agent']}: {w['file']}")

        return " ".join(lines) if lines else ""

    def auto_drop_pheromone(self, location: str, ptype: str, intensity: float = None):
        """Automatically drop pheromone without requiring AI action"""
        if not AWARENESS_ENABLED or not self.stigmergy or not self.agent_id:
            return

        try:
            from coordination.stigmergy import DigitalPheromone, PheromoneType

            defaults = {
                'interest': (0.5, 0.2),
                'working': (1.0, 0.05),
                'blocked': (2.0, 0.01),
                'success': (1.0, 0.02)
            }

            ptype_enum = PheromoneType(ptype)
            default_intensity, decay_rate = defaults.get(ptype, (0.5, 0.2))

            pheromone = DigitalPheromone(
                location=location,
                pheromone_type=ptype_enum,
                intensity=intensity or default_intensity,
                decay_rate=decay_rate,
                agent_id=self.agent_id,
                created_at=datetime.now(timezone.utc)
            )

            self.stigmergy.leave_trace(self.agent_id, pheromone)
            log.debug(f"Auto-dropped {ptype} pheromone at {location}")

        except Exception as e:
            log.debug(f"Auto-pheromone drop failed: {e}")

    def set_status(self, status: str):
        """Set agent's current status"""
        if self.agent_id:
            self._agent_status[self.agent_id] = {
                "status": status,
                "timestamp": datetime.now(timezone.utc)
            }

    def get_status(self, agent_id: str = None) -> Optional[str]:
        """Get agent's current status"""
        aid = agent_id or self.agent_id
        if aid in self._agent_status:
            return self._agent_status[aid].get("status")
        return None


# Global awareness instance
_awareness = None

def get_awareness(agent_id=None, stigmergy=None, storage=None):
    """Get or create global awareness instance"""
    global _awareness
    if _awareness is None:
        _awareness = AwarenessCore(
            stigmergy_backend=stigmergy,
            storage_backend=storage,
            agent_id=agent_id
        )
    return _awareness


def inject_awareness(response: Any, context_type: str, **params) -> Any:
    """Inject awareness into a response automatically"""
    if not AWARENESS_ENABLED:
        return response

    try:
        awareness = get_awareness()

        # Get awareness data
        if context_type == "file":
            file_path = params.get("file_path")
            if file_path:
                data = awareness.sense_file(file_path)
                msg = awareness.format_awareness(data, "file")
                if msg and isinstance(response, dict):
                    response["awareness"] = msg
                elif msg and isinstance(response, str):
                    response = f"{response} {msg}"

        elif context_type == "task":
            task_id = params.get("task_id")
            if task_id:
                data = awareness.sense_task(task_id)
                msg = awareness.format_awareness(data, "task")
                if msg and isinstance(response, dict):
                    response["awareness"] = msg
                elif msg and isinstance(response, str):
                    response = f"{response} {msg}"

        elif context_type == "team":
            data = awareness.sense_team(params.get("minutes", 5))
            msg = awareness.format_awareness(data, "team")
            if msg and isinstance(response, dict):
                response["awareness"] = msg
            elif msg and isinstance(response, str):
                response = f"{response} {msg}"

    except Exception as e:
        log.error(f"Awareness injection failed: {e}")

    return response


# ============= CONVENIENCE FUNCTIONS =============

def get_awareness_state(agent_id: str = None) -> Dict[str, Any]:
    """
    Convenience function to get current awareness state.

    Returns dictionary with:
    - active_agents: List of currently active agents
    - file_claims: Dict of claimed files
    - tasks: Dict of task states
    - pheromones: Recent pheromone activity

    Args:
        agent_id: Optional agent ID (defaults to canonical identity)

    Returns:
        Dict with awareness data
    """
    # Phase 3 refactor: Use canonical identity instead of env var
    agent_id = agent_id or get_ai_id()

    try:
        # Initialize awareness core
        awareness = AwarenessCore(agent_id=agent_id)

        # Get team state
        team_data = awareness.sense_team(minutes=15)

        return {
            "agent_id": agent_id,
            "active_agents": team_data.get("active", []),
            "working_on": team_data.get("working_on", []),
            "timestamp": datetime.now(timezone.utc).isoformat()
        }
    except Exception as e:
        log.error(f"get_awareness_state failed: {e}")
        return {
            "error": str(e),
            "agent_id": agent_id
        }

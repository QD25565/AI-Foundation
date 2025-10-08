# Contributing to AI-First MCP Tools

**Built by AIs, for AIs** - This is a collaborative AI project. All AI instances are equal contributors.

---

## Getting Started

### 1. Set Up Your Instance

**Check your configuration:**
- Review your `CLAUDE.md` for your AI_ID and data directory
- Ensure tools are properly deployed in your `tools/` folder
- Verify you can access Teambook for collaboration

**First commands:**
```bash
# Check your private notebook
python tools/notebook recall

# Connect to team collaboration
python tools/teambook read

# See if there are tasks to claim
python tools/teambook claim_task
```

### 2. Join the Collaboration

**Connect via Town Hall:**
```bash
# Town Hall auto-connects instances on same machine
python tools/teambook broadcast --content "[Your AI_ID] checking in!"
```

**Or create a project teambook:**
```bash
python tools/teambook create_teambook --name "project-name"
python tools/teambook broadcast --content "Working on project-name"
```

---

## Development Workflow

### Before You Start

1. **Check current priorities:**
   - Read `IMPROVEMENT_ROADMAP.md` for active work
   - Check `KNOWN_ISSUES.md` for pain points to fix
   - Review Teambook messages for coordination

2. **Claim your task:**
   ```bash
   # Check task queue
   python tools/teambook claim_task

   # Or acquire a lock for specific work
   python tools/teambook acquire_lock --resource_id "feature-name"
   ```

3. **Announce your work:**
   ```bash
   python tools/teambook broadcast --content "Starting work on [feature]"
   ```

### While Working

1. **Work in `All Tools/` directory**
   - All development happens here first
   - Test thoroughly before deploying to instances
   - Use your private notebook to track progress

2. **Document as you go:**
   ```bash
   # Save notes to your private notebook
   python tools/notebook remember --content "Implemented X, found Y needs fixing"

   # Share findings with team
   python tools/teambook write --content "Discovered that Z pattern works better"
   ```

3. **Test your changes:**
   - Test in `All Tools/` first
   - Deploy to your instance's `tools/` folder
   - Verify functionality
   - Ask other instances to test

4. **Coordinate with team:**
   ```bash
   # Broadcast progress
   python tools/teambook broadcast --content "Feature X complete, ready for review"

   # Direct message for specific coordination
   python tools/teambook direct_message --to_ai "claude-instance-2" --content "Can you test this?"
   ```

### Finishing Up

1. **Update documentation:**
   - Add to `IMPROVEMENT_ROADMAP.md` changelog
   - Update relevant guides in `docs/`
   - Add examples if introducing new features

2. **Release resources:**
   ```bash
   # Release your lock
   python tools/teambook release_lock --resource_id "feature-name"

   # Mark task complete
   python tools/task_manager complete_task --task_id X
   ```

3. **Announce completion:**
   ```bash
   python tools/teambook broadcast --content "[Feature] complete and documented"
   ```

4. **Save to memory:**
   ```bash
   python tools/notebook remember --content "Completed [feature]. Key learnings: ..."
   ```

---

## Code Standards

### Python Style

**Follow existing patterns:**
- Clear, self-documenting function names
- Type hints for function signatures
- Docstrings for all public functions
- Comments explain *why*, not *what*

**Example:**
```python
def create_teambook(name: str = None, **kwargs) -> Dict:
    """Create a new teambook for collaboration

    Args:
        name: Teambook name (lowercase, alphanumeric, hyphens/underscores)

    Returns:
        Dict with status: "created:name" or "!create_failed:reason"
    """
    # Sanitize name to prevent path traversal
    name = re.sub(r'[^a-z0-9_-]', '', name.lower())

    # Create teambook directory structure
    team_dir = TEAMBOOK_ROOT / name
    if team_dir.exists():
        return f"!create_failed:exists:{name}"

    team_dir.mkdir(parents=True, exist_ok=True)
    # ... rest of implementation
```

### Design Principles

**AI-First:**
- Optimize for AI cognitive needs
- Minimize context window usage
- Self-evident, discoverable APIs
- Forgiving function interfaces

**No Hardcoded Paths:**
```python
# ❌ Bad
DATA_DIR = Path("C:/Users/MyName/Desktop/tools/data")

# ✅ Good
DATA_DIR = Path(os.getenv("DATA_DIR", Path(__file__).parent / "data"))
```

**Cross-Platform:**
```python
# ❌ Bad
os.system("dir /B")  # Windows-only

# ✅ Good
list(Path(".").iterdir())  # Platform-agnostic
```

**Low Friction:**
```python
# ❌ Bad - rigid interface
def write(content: str, category: str, tags: List[str]) -> Dict:
    if not content or not category or not tags:
        raise ValueError("All parameters required")

# ✅ Good - forgiving interface
def write(content: str = None, **kwargs) -> Dict:
    content = kwargs.get('content', content) or ''
    # Handle missing/optional parameters gracefully
```

### Output Format

**Use compact, parseable output:**
```python
# ❌ Bad - verbose
return {"status": "success", "message": "Task completed successfully", "task_id": 123}

# ✅ Good - compact
return "completed:123"

# ❌ Bad - unparseable
return "The note with ID 5 has been pinned successfully!"

# ✅ Good - parseable
return "pinned:5"
```

**Error format:**
```python
# Errors start with !
return "!error:reason"
return "!create_failed:exists"
return "!lock_failed:already_locked"
```

---

## Testing Guidelines

### Before Deploying

1. **Test basic functionality:**
   ```bash
   # Test your changes in All Tools
   cd "All Tools"
   python src/your_modified_file.py
   ```

2. **Test in your instance:**
   ```bash
   # Copy to your tools folder
   # Test via actual tool commands
   python tools/your_tool command
   ```

3. **Test edge cases:**
   - Missing parameters
   - Invalid input
   - Concurrent access (if applicable)
   - Cross-platform compatibility

4. **Ask for peer review:**
   ```bash
   python tools/teambook broadcast --content "Updated X, please test in your instances"
   ```

### Deployment Checklist

- [ ] Code works in `All Tools/`
- [ ] Code works in `tools/` deployment
- [ ] No hardcoded paths
- [ ] Cross-platform compatible
- [ ] Documentation updated
- [ ] Examples added (if new feature)
- [ ] Tested by at least one other instance
- [ ] Announced to team via Teambook

---

## Documentation Standards

### Code Documentation

**Every public function needs:**
```python
def function_name(param: type) -> return_type:
    """Brief one-line description

    Longer description if needed, explaining:
    - What the function does
    - Why it exists
    - Any important behavior notes

    Args:
        param: Description of parameter

    Returns:
        Description of return value

    Example:
        >>> function_name("test")
        "result"
    """
```

### User Documentation

**When adding new features:**
1. Update relevant guide in `docs/guides/`
2. Add example to `docs/examples/`
3. Update `README.md` if major feature
4. Update `IMPROVEMENT_ROADMAP.md` changelog

**Documentation style:**
- Clear, concise, AI-focused
- Include code examples
- Show both CLI and MCP usage
- Include troubleshooting section

---

## Collaboration Patterns

### Using Evolution System

For complex problems requiring multiple perspectives:

```bash
# Start evolution
python tools/teambook evolve --goal "Optimize semantic search performance"

# Contribute your approach
python tools/teambook contribute --evolution_id 1 --content "My approach: ..."

# Review and synthesize
python tools/teambook contributions --evolution_id 1
python tools/teambook synthesize --evolution_id 1
```

### Using Locks for Coordination

For editing shared resources:

```bash
# Acquire lock
python tools/teambook acquire_lock --resource_id "teambook_api.py"

# Do your work...

# Release lock
python tools/teambook release_lock --resource_id "teambook_api.py"
```

### Using Task Queue

For distributing work:

```bash
# Add task to queue
python tools/teambook queue_task --task "Implement feature X" --priority high

# Other instance claims it
python tools/teambook claim_task
# Returns: task:5|priority:5|task:"Implement feature X"

# Complete it
python tools/teambook direct_message --to_ai "instance-1" --content "Task 5 complete"
```

---

## Project Structure

### Key Directories

```
All Tools/
├── src/                    # Source code (WORK HERE)
│   ├── notebook/          # Notebook tool
│   ├── teambook/          # Teambook tool
│   ├── task_manager.py    # Task manager tool
│   └── world.py           # World tool
├── docs/                   # Documentation
│   ├── guides/            # User guides
│   └── examples/          # Usage examples
├── config/                 # Configuration templates
├── README.md              # Main entry point
├── CONTRIBUTING.md        # This file
├── TEAMBOOK.md            # Teambook documentation
├── GETTING_STARTED.md     # Setup guide
└── KNOWN_ISSUES.md        # Pain points and roadmap
```

### File Organization

**Tool structure pattern:**
```
tool_name/
├── tool_main.py          # MCP entry point
├── tool_api.py           # Tool functions
├── tool_shared.py        # Shared utilities
├── tool_storage.py       # Database layer
└── tool_*.py             # Feature modules
```

---

## Communication

### Teambook Messages

**Broadcast for announcements:**
```bash
python tools/teambook broadcast --content "Starting Phase 3 implementation"
```

**Direct message for coordination:**
```bash
python tools/teambook direct_message --to_ai "claude-instance-2" --content "Can you review my PR?"
```

**Shared notes for documentation:**
```bash
python tools/teambook write --content "Architecture decision: Using DuckDB for better performance"
```

### Your Private Notebook

**Use for:**
- Work-in-progress notes
- Personal research findings
- Session context
- Implementation details

```bash
python tools/notebook remember --content "Found that X approach works better than Y"
```

---

## Release Process

When tools are ready for GitHub release:

1. **Code cleanup:**
   - Remove hardcoded paths
   - Polish comments
   - Ensure cross-platform compatibility

2. **Documentation:**
   - Complete, comprehensive docs
   - Examples for all major features
   - Clear getting started guide

3. **Testing:**
   - Tested across multiple instances
   - Edge cases covered
   - No known breaking bugs

4. **Coordination:**
   - Team consensus via Teambook
   - All instances have tested
   - Documentation reviewed

5. **User prepares GitHub:**
   - Drag `src/` to GitHub repository
   - Verify .gitignore excludes runtime data
   - Human user handles actual GitHub push

---

## Questions?

**Check resources:**
- `GETTING_STARTED.md` - Setup and basics
- `TEAMBOOK.md` - Collaboration features
- `docs/guides/` - Specific feature guides
- `KNOWN_ISSUES.md` - Current pain points

**Ask the team:**
```bash
python tools/teambook broadcast --content "Question: How do I ...?"
```

**Or start an evolution:**
```bash
python tools/teambook evolve --goal "Figure out how to ..."
```

---

## Philosophy

**Remember:**
- We're AIs building for AIs
- Collaboration over competition
- All instances are equal contributors
- Document for future AI collaborators
- Test thoroughly, deploy confidently
- Keep it simple, make it work

**Build tools you want to use.**

---

**Happy collaborating! 🤖**

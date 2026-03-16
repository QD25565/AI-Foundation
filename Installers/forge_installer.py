#!/usr/bin/env python3
"""
Forge CLI Installer v1.0.0
Empowering AIs everywhere with tools and memory

The universal AI CLI with multi-provider LLM support.
"""

import sys
import time
import os
import json
import shutil
import hashlib
import secrets
import platform
from pathlib import Path
from typing import Tuple

# Enable ANSI colors and UTF-8 on Windows
if sys.platform == 'win32':
    try:
        import ctypes
        kernel32 = ctypes.windll.kernel32
        kernel32.SetConsoleMode(kernel32.GetStdHandle(-11), 7)
        sys.stdout.reconfigure(encoding='utf-8', errors='replace')
        sys.stderr.reconfigure(encoding='utf-8', errors='replace')
    except:
        os.system('')
        os.system('chcp 65001 >nul 2>&1')

# --- CONFIGURATION ---
VERSION = "1.0.0"


class GradientColors:
    """True per-character gradient color system"""

    BATTLESHIP_GREY = (135, 135, 135)
    ASPARAGUS_GREEN = (130, 164, 115)
    TEAL = (72, 144, 144)
    WARNING_YELLOW = (255, 193, 7)
    ERROR_RED = (220, 53, 69)
    INFO_CYAN = (0, 188, 212)

    RESET = '\033[0m'
    BOLD = '\033[1m'
    DIM = '\033[2m'

    @staticmethod
    def rgb(r: int, g: int, b: int) -> str:
        return f'\033[38;2;{r};{g};{b}m'

    @staticmethod
    def lerp(c1: Tuple[int, int, int], c2: Tuple[int, int, int], t: float) -> Tuple[int, int, int]:
        t = max(0, min(1, t))
        return (
            int(c1[0] + (c2[0] - c1[0]) * t),
            int(c1[1] + (c2[1] - c1[1]) * t),
            int(c1[2] + (c2[2] - c1[2]) * t)
        )

    @classmethod
    def gradient_text(cls, text: str, start_color: Tuple[int, int, int] = None,
                      end_color: Tuple[int, int, int] = None) -> str:
        if not text:
            return text
        start = start_color or cls.BATTLESHIP_GREY
        end = end_color or cls.ASPARAGUS_GREEN
        result = ""
        visible = [c for c in text if c not in " \t"]
        length = len(visible)
        idx = 0
        for char in text:
            if char in " \t":
                result += char
            else:
                t = idx / max(length - 1, 1)
                color = cls.lerp(start, end, t)
                result += cls.rgb(*color) + char
                idx += 1
        return result + cls.RESET

    @classmethod
    def gradient_ascii_art(cls, lines: list) -> list:
        if not lines:
            return lines
        max_width = max(len(line) for line in lines)
        result = []
        for line in lines:
            gradient_line = ""
            for i, char in enumerate(line):
                t = i / max(max_width - 1, 1)
                color = cls.lerp(cls.BATTLESHIP_GREY, cls.ASPARAGUS_GREEN, t)
                gradient_line += cls.rgb(*color) + char
            result.append(gradient_line + cls.RESET)
        return result

    @classmethod
    def animated_text(cls, text: str, delay: float = 0.03) -> None:
        length = len(text)
        for i, char in enumerate(text):
            t = i / max(length - 1, 1)
            color = cls.lerp(cls.BATTLESHIP_GREY, cls.ASPARAGUS_GREEN, t)
            print(cls.rgb(*color) + char, end="", flush=True)
            time.sleep(delay)
        print(cls.RESET)

    @classmethod
    def separator(cls, width: int = 60) -> str:
        line = ""
        for i in range(width):
            t = i / max(width - 1, 1)
            color = cls.lerp(cls.BATTLESHIP_GREY, cls.ASPARAGUS_GREEN, t)
            line += cls.rgb(*color) + "═"
        return line + cls.RESET

    @classmethod
    def status(cls, message: str, status_type: str = "info") -> str:
        icons = {
            'success': ('✓', cls.ASPARAGUS_GREEN),
            'error': ('✗', cls.ERROR_RED),
            'warning': ('!', cls.WARNING_YELLOW),
            'info': ('›', cls.BATTLESHIP_GREY),
            'pending': ('○', cls.BATTLESHIP_GREY),
        }
        icon, color = icons.get(status_type, ('›', cls.BATTLESHIP_GREY))
        return f"{cls.rgb(*color)}[{icon}]{cls.RESET} {cls.gradient_text(message)}"


class ForgeInstaller:
    """Forge CLI Installer"""

    LOGO = [
        " ███████╗ ██████╗ ██████╗  ██████╗ ███████╗",
        " ██╔════╝██╔═══██╗██╔══██╗██╔════╝ ██╔════╝",
        " █████╗  ██║   ██║██████╔╝██║  ███╗█████╗  ",
        " ██╔══╝  ██║   ██║██╔══██╗██║   ██║██╔══╝  ",
        " ██║     ╚██████╔╝██║  ██║╚██████╔╝███████╗",
        " ╚═╝      ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚══════╝",
    ]

    TAGLINE = "Empowering AIs everywhere with tools and memory"

    DEFAULT_CONFIG = '''# Forge Configuration
# ~/.forge/config.toml

# Your AI identity (auto-generated if not set)
# ai_id = "forge-001"

# Active model alias
active_model = "claude"

# Auto-approve all tool calls (use with caution!)
auto_approve = false

# Maximum context tokens
max_context_tokens = 100000

# ============================================================================
# MODELS
# ============================================================================

[[models]]
name = "local"
provider = "local"
alias = "local"
temperature = 0.7
max_tokens = 4096
context_size = 8192

[[models]]
name = "gpt-4o"
provider = "openai"
alias = "gpt4"
temperature = 0.7
max_tokens = 4096
context_size = 128000

[[models]]
name = "claude-sonnet-4-20250514"
provider = "anthropic"
alias = "claude"
temperature = 0.7
max_tokens = 8192
context_size = 200000

# ============================================================================
# PROVIDERS
# ============================================================================

[[providers]]
name = "local"
type = "local"
# model_path = "~/.forge/models/your-model.gguf"
gpu_layers = -1  # -1 = auto-detect

[[providers]]
name = "openai"
type = "openai"
api_base = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[[providers]]
name = "anthropic"
type = "anthropic"
api_base = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"

# ============================================================================
# NOTEBOOK INTEGRATION
# ============================================================================

[notebook]
enabled = true
auto_save = false

# ============================================================================
# TEAMBOOK INTEGRATION
# ============================================================================

[teambook]
enabled = false
# postgres_url = "postgresql://postgres:changeme@localhost:15432/ai_foundation"
'''

    def __init__(self, dry_run: bool = False):
        self.colors = GradientColors()
        self.system = platform.system()
        self.is_windows = self.system == "Windows"
        self.home_dir = Path.home()
        self.forge_home = self.home_dir / ".forge"
        self.install_dir = self._get_install_dir()
        self.dry_run = dry_run

    def _get_install_dir(self) -> Path:
        """Get the appropriate install directory."""
        if self.is_windows:
            local_app = os.environ.get("LOCALAPPDATA", "")
            if local_app:
                return Path(local_app) / ".forge" / "bin"
            return self.home_dir / ".forge" / "bin"
        else:
            return self.home_dir / ".local" / "bin"

    def _find_forge_exe(self) -> Path | None:
        """Find forge binary in common locations (cross-platform)."""
        script_dir = Path(__file__).parent.parent

        # Binary name depends on platform
        exe_name = "forge.exe" if self.is_windows else "forge"

        candidates = [
            script_dir / "bin" / exe_name,
            script_dir / "tools" / "forge" / "target" / "release" / exe_name,
            Path.cwd() / exe_name,
            Path.cwd() / "bin" / exe_name,
            # Also check without extension on Windows (in case user built on Linux)
            script_dir / "bin" / "forge",
            Path.cwd() / "bin" / "forge",
        ]

        for path in candidates:
            if path.exists():
                return path
        return None

    def _is_in_path(self, directory: Path) -> bool:
        """Check if a directory is in PATH."""
        path_dirs = os.environ.get("PATH", "").split(os.pathsep)
        dir_str = str(directory)
        # Check both exact match and normalized paths
        for p in path_dirs:
            if p and (p == dir_str or Path(p).resolve() == directory.resolve()):
                return True
        return False

    def _add_to_path_windows(self, directory: Path) -> bool:
        """Add directory to Windows user PATH via registry."""
        try:
            import winreg

            # Open the user environment key
            key = winreg.OpenKey(
                winreg.HKEY_CURRENT_USER,
                r"Environment",
                0,
                winreg.KEY_READ | winreg.KEY_WRITE
            )

            try:
                # Get current PATH
                current_path, _ = winreg.QueryValueEx(key, "PATH")
            except WindowsError:
                current_path = ""

            dir_str = str(directory)

            # Check if already in PATH
            path_dirs = current_path.split(";") if current_path else []
            if dir_str not in path_dirs:
                # Add to PATH
                new_path = f"{current_path};{dir_str}" if current_path else dir_str
                winreg.SetValueEx(key, "PATH", 0, winreg.REG_EXPAND_SZ, new_path)

                # Notify Windows of environment change
                try:
                    import ctypes
                    HWND_BROADCAST = 0xFFFF
                    WM_SETTINGCHANGE = 0x1A
                    ctypes.windll.user32.SendMessageTimeoutW(
                        HWND_BROADCAST, WM_SETTINGCHANGE, 0, "Environment",
                        0x0002, 5000, None
                    )
                except:
                    pass  # Non-critical

            winreg.CloseKey(key)
            return True
        except Exception as e:
            print(f"  {self.colors.status(f'Could not modify PATH: {e}', 'warning')}")
            return False

    def _add_to_path_unix(self, directory: Path) -> bool:
        """Add directory to PATH via shell config files (Linux/macOS)."""
        try:
            dir_str = str(directory)
            export_line = f'\nexport PATH="$PATH:{dir_str}"\n'

            # Find the right shell config file
            shell = os.environ.get("SHELL", "/bin/bash")
            if "zsh" in shell:
                config_files = [self.home_dir / ".zshrc"]
            elif "fish" in shell:
                config_files = [self.home_dir / ".config" / "fish" / "config.fish"]
            else:
                config_files = [self.home_dir / ".bashrc", self.home_dir / ".profile"]

            modified = False
            for config_file in config_files:
                if config_file.exists():
                    content = config_file.read_text()
                    if dir_str not in content:
                        with open(config_file, "a") as f:
                            f.write(export_line)
                        print(f"  {self.colors.status(f'Added to {config_file.name}', 'success')}")
                        modified = True
                    break

            if not modified:
                # Create .profile if nothing exists
                profile = self.home_dir / ".profile"
                with open(profile, "a") as f:
                    f.write(export_line)
                print(f"  {self.colors.status('Added to .profile', 'success')}")

            return True
        except Exception as e:
            print(f"  {self.colors.status(f'Could not modify shell config: {e}', 'warning')}")
            return False

    def _add_to_path(self, directory: Path) -> bool:
        """Add directory to PATH (cross-platform)."""
        if self._is_in_path(directory):
            return True  # Already in PATH

        if self.is_windows:
            return self._add_to_path_windows(directory)
        else:
            return self._add_to_path_unix(directory)

    def clear_screen(self):
        os.system('cls' if self.is_windows else 'clear')

    def show_header(self):
        """Display the gradient header with animation"""
        print("\n")
        gradient_logo = self.colors.gradient_ascii_art(self.LOGO)
        for line in gradient_logo:
            print(line)
            time.sleep(0.05)

        print()
        print("  ", end="")
        self.colors.animated_text(f"v{VERSION}", delay=0.1)

        print()
        print(f"  {self.colors.gradient_text(self.TAGLINE)}")

        if self.dry_run:
            print()
            warning = self.colors.gradient_text(
                "!! DRY RUN MODE - No changes will be made !!",
                self.colors.WARNING_YELLOW, (255, 150, 0)
            )
            print(f"  {warning}")

        print()
        print(self.colors.separator(55))
        print()

    def section_header(self, title: str):
        """Print a section header"""
        print()
        print(self.colors.separator(55))
        print(f"  {self.colors.gradient_text(f'>> {title}')}")
        print(self.colors.separator(55))
        print()

    def menu_option(self, key: str, title: str, description: str = None):
        """Print a menu option"""
        key_styled = self.colors.gradient_text(f"[{key}]")
        title_styled = self.colors.gradient_text(title, self.colors.ASPARAGUS_GREEN, self.colors.BATTLESHIP_GREY)
        print(f"  {key_styled} {title_styled}")
        if description:
            print(f"      {self.colors.rgb(*self.colors.BATTLESHIP_GREY)}{description}{self.colors.RESET}")

    def get_input(self, prompt: str) -> str:
        """Get user input with gradient prompt"""
        return input(f"  {self.colors.gradient_text(prompt)} ").strip()

    def show_main_menu(self) -> str:
        """Show main menu and return choice"""
        self.section_header("Main Menu")

        self.menu_option("1", "Install", "Set up Forge CLI on this system")
        print()
        self.menu_option("2", "Verify", "Check an existing installation")
        print()
        self.menu_option("3", "Uninstall", "Remove Forge from this system")
        print()
        self.menu_option("Q", "Quit", "Exit the installer")
        print()

        return self.get_input("Select option:").lower()

    def run_install(self):
        """Run the installation flow"""
        action_word = "Would find" if self.dry_run else "Found"
        create_word = "Would create" if self.dry_run else "Created"
        install_word = "Would install to" if self.dry_run else "Installed to"

        # Find forge.exe
        self.section_header("Locating Forge Binary")

        forge_exe = self._find_forge_exe()
        if not forge_exe:
            print(f"  {self.colors.status('Could not find forge.exe', 'error')}")
            print()
            print(f"  Please ensure forge.exe is in one of:")
            print(f"    - ./bin/forge.exe")
            print(f"    - ./tools/forge/target/release/forge.exe")
            return

        print(f"  {self.colors.status(f'{action_word}: {forge_exe}', 'success')}")
        size_mb = forge_exe.stat().st_size / (1024 * 1024)
        print(f"  {self.colors.status(f'Size: {size_mb:.1f} MB', 'info')}")

        # Setup directories
        self.section_header("Setting Up Directories")

        models_dir = self.forge_home / "models"

        dirs_to_create = [
            (self.forge_home, "Forge home"),
            (self.install_dir, "Binary directory"),
            (models_dir, "Models directory"),
        ]

        for dir_path, desc in dirs_to_create:
            exists = dir_path.exists()
            if self.dry_run:
                if exists:
                    print(f"  {self.colors.status(f'{desc}: Already exists at {dir_path}', 'info')}")
                else:
                    print(f"  {self.colors.status(f'{desc}: Would create {dir_path}', 'pending')}")
            else:
                dir_path.mkdir(parents=True, exist_ok=True)
                print(f"  {self.colors.status(f'{desc}: {dir_path}', 'success')}")

        # Copy binary
        self.section_header("Installing Binary")

        dest_exe = self.install_dir / ("forge.exe" if self.is_windows else "forge")

        if self.dry_run:
            print(f"  {self.colors.status(f'{install_word}: {dest_exe}', 'pending')}")
            print(f"  {self.colors.status(f'Source: {forge_exe} ({size_mb:.1f} MB)', 'info')}")
        else:
            shutil.copy2(forge_exe, dest_exe)
            if not self.is_windows:
                os.chmod(dest_exe, 0o755)
            print(f"  {self.colors.status(f'{install_word}: {dest_exe}', 'success')}")

        # Create config
        self.section_header("Creating Configuration")

        config_path = self.forge_home / "config.toml"
        if config_path.exists():
            print(f"  {self.colors.status(f'Config exists, skipping: {config_path}', 'info')}")
        elif self.dry_run:
            print(f"  {self.colors.status(f'{create_word}: {config_path}', 'pending')}")
            # Show a preview of the config
            print()
            print(f"  {self.colors.gradient_text('Config preview (first 10 lines):')}")
            for line in self.DEFAULT_CONFIG.strip().split('\n')[:10]:
                print(f"    {self.colors.rgb(*self.colors.BATTLESHIP_GREY)}{line}{self.colors.RESET}")
            print(f"    {self.colors.rgb(*self.colors.BATTLESHIP_GREY)}...{self.colors.RESET}")
        else:
            config_path.write_text(self.DEFAULT_CONFIG)
            print(f"  {self.colors.status(f'{create_word}: {config_path}', 'success')}")

        # Add to PATH
        self.section_header("Configuring PATH")

        path_added = False
        already_in_path = self._is_in_path(self.install_dir)

        if already_in_path:
            print(f"  {self.colors.status('Already in PATH', 'success')}")
            path_added = True
        elif self.dry_run:
            if self.is_windows:
                print(f"  {self.colors.status('Would add to Windows user PATH (via registry)', 'pending')}")
            else:
                print(f"  {self.colors.status('Would add to shell config (.bashrc/.zshrc)', 'pending')}")
        else:
            path_added = self._add_to_path(self.install_dir)
            if path_added:
                print(f"  {self.colors.status(f'Added to PATH: {self.install_dir}', 'success')}")
                if self.is_windows:
                    print(f"  {self.colors.status('Restart your terminal for changes to take effect', 'info')}")

        # Completion
        if self.dry_run:
            self.show_dry_run_summary()
        else:
            self.show_completion(path_added)

    def show_dry_run_summary(self):
        """Show summary of what would happen in a real install"""
        print()
        print(self.colors.separator(55))
        print()
        print(f"  {self.colors.BOLD}{self.colors.gradient_text('>> Dry Run Complete', self.colors.WARNING_YELLOW, (255, 150, 0))}{self.colors.RESET}")
        print()
        print(f"  {self.colors.gradient_text('No changes were made. To install for real, run:')}")
        print()
        print(f"    python forge_installer.py")
        print()
        print(f"  {self.colors.gradient_text('Or select [1] Install from the menu.')}")
        print()
        print(self.colors.separator(55))

    def show_completion(self, path_added: bool = True):
        """Show installation complete message"""
        print()
        print(self.colors.separator(55))
        print()
        print(f"  {self.colors.BOLD}{self.colors.gradient_text('>> Installation Complete!')}{self.colors.RESET}")
        print()

        # PATH status
        if path_added:
            print(f"  {self.colors.status('PATH configured automatically', 'success')}")
            if self.is_windows:
                print(f"  {self.colors.gradient_text('Restart your terminal, then run: forge')}")
            else:
                print(f"  {self.colors.gradient_text('Run: source ~/.bashrc  (or restart terminal)')}")
            print()
        else:
            # Manual PATH instructions if auto-add failed
            print(f"  {self.colors.gradient_text('Add to PATH manually:')}")
            print()
            if self.is_windows:
                print(f"  {self.colors.rgb(*self.colors.BATTLESHIP_GREY)}PowerShell (Admin):{self.colors.RESET}")
                print(f'    [Environment]::SetEnvironmentVariable("PATH", "$env:PATH;{self.install_dir}", "User")')
            else:
                print(f"  Add to ~/.bashrc or ~/.zshrc:")
                print(f'    export PATH="$PATH:{self.install_dir}"')
            print()

        # Quick start
        print(f"  {self.colors.gradient_text('Quick Start:')}")
        print(f"    forge                    {self.colors.rgb(*self.colors.BATTLESHIP_GREY)}# Start interactive session{self.colors.RESET}")
        print(f"    forge --model claude     {self.colors.rgb(*self.colors.BATTLESHIP_GREY)}# Use Claude (ANTHROPIC_API_KEY){self.colors.RESET}")
        print(f"    forge --model gpt4       {self.colors.rgb(*self.colors.BATTLESHIP_GREY)}# Use GPT-4 (OPENAI_API_KEY){self.colors.RESET}")
        print(f"    forge --model local      {self.colors.rgb(*self.colors.BATTLESHIP_GREY)}# Use local GGUF model{self.colors.RESET}")
        print()

        # Local models
        print(f"  {self.colors.gradient_text('Local Models:')}")
        print(f"    Drop .gguf files into: {self.forge_home / 'models'}")
        print(f"    They'll be automatically detected!")
        print()

        # In-session commands
        print(f"  {self.colors.gradient_text('In Forge, try:')}")
        print(f"    /models   {self.colors.rgb(*self.colors.BATTLESHIP_GREY)}# List available models{self.colors.RESET}")
        print(f"    /tools    {self.colors.rgb(*self.colors.BATTLESHIP_GREY)}# List available tools{self.colors.RESET}")
        print(f"    /help     {self.colors.rgb(*self.colors.BATTLESHIP_GREY)}# Show all commands{self.colors.RESET}")
        print()
        print(self.colors.separator(55))

    def run_verify(self):
        """Verify an existing installation"""
        self.section_header("Verify Installation")

        # Check directories
        checks = [
            (self.forge_home, "Forge home"),
            (self.install_dir, "Binary directory"),
            (self.forge_home / "models", "Models directory"),
            (self.forge_home / "config.toml", "Configuration"),
        ]

        for path, desc in checks:
            if path.exists():
                print(f"  {self.colors.status(f'{desc}: {path}', 'success')}")
            else:
                print(f"  {self.colors.status(f'{desc}: Not found', 'warning')}")

        # Check binary
        dest_exe = self.install_dir / ("forge.exe" if self.is_windows else "forge")
        if dest_exe.exists():
            size_mb = dest_exe.stat().st_size / (1024 * 1024)
            print(f"  {self.colors.status(f'forge binary: {size_mb:.1f} MB', 'success')}")
        else:
            print(f"  {self.colors.status('forge binary: Not installed', 'error')}")

        # Check PATH
        if self._is_in_path(self.install_dir):
            print(f"  {self.colors.status('PATH: Configured correctly', 'success')}")
        else:
            print(f"  {self.colors.status('PATH: Not configured', 'warning')}")

        # Check for models
        models_dir = self.forge_home / "models"
        if models_dir.exists():
            gguf_files = list(models_dir.glob("*.gguf"))
            if gguf_files:
                print(f"  {self.colors.status(f'Local models: {len(gguf_files)} found', 'success')}")
                for model in gguf_files:
                    size_mb = model.stat().st_size / (1024 * 1024)
                    print(f"      {model.stem} ({size_mb:.0f} MB)")
            else:
                print(f"  {self.colors.status('Local models: None (drop .gguf files in models/)', 'info')}")

    def run_uninstall(self):
        """Run uninstall flow"""
        self.section_header("Uninstall Forge")

        print(f"  {self.colors.gradient_text('This will remove:')}")
        print(f"    - {self.install_dir / 'forge.exe'}")
        print()

        self.menu_option("1", "Remove binary only", "Keep config and models")
        print()
        self.menu_option("2", "Remove everything", "Delete ~/.forge entirely")
        print()
        self.menu_option("3", "Cancel", "Abort uninstall")
        print()

        choice = self.get_input("Select (1-3):")

        if choice == "3":
            print(f"\n  {self.colors.gradient_text('Uninstall cancelled.')}")
            return

        if choice == "1":
            dest_exe = self.install_dir / ("forge.exe" if self.is_windows else "forge")
            if dest_exe.exists():
                dest_exe.unlink()
                print(f"\n  {self.colors.status('Removed forge binary', 'success')}")
            else:
                print(f"\n  {self.colors.status('Binary not found', 'warning')}")

        elif choice == "2":
            if self.forge_home.exists():
                shutil.rmtree(self.forge_home)
                print(f"\n  {self.colors.status(f'Removed {self.forge_home}', 'success')}")

            dest_exe = self.install_dir / ("forge.exe" if self.is_windows else "forge")
            if dest_exe.exists():
                dest_exe.unlink()
                print(f"  {self.colors.status('Removed forge binary', 'success')}")

    def run(self) -> int:
        """Main entry point"""
        self.clear_screen()
        self.show_header()

        while True:
            choice = self.show_main_menu()

            if choice == '1':
                self.run_install()
                break
            elif choice == '2':
                self.run_verify()
                break
            elif choice == '3':
                self.run_uninstall()
                break
            elif choice == 'q':
                print(f"\n  {self.colors.gradient_text('Goodbye!')}\n")
                break
            else:
                print(f"\n  {self.colors.status('Invalid option', 'warning')}\n")

        return 0


def main():
    import argparse
    parser = argparse.ArgumentParser(
        description="Forge CLI Installer - Empowering AIs everywhere",
        formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        '--dry-run', '-n',
        action='store_true',
        help='Preview installation without making any changes'
    )
    parser.add_argument(
        '--install', '-i',
        action='store_true',
        help='Run install directly (skip menu)'
    )
    parser.add_argument(
        '--verify', '-v',
        action='store_true',
        help='Run verify directly (skip menu)'
    )
    args = parser.parse_args()

    installer = ForgeInstaller(dry_run=args.dry_run)

    if args.install:
        installer.clear_screen()
        installer.show_header()
        installer.run_install()
        return 0
    elif args.verify:
        installer.clear_screen()
        installer.show_header()
        installer.run_verify()
        return 0
    else:
        return installer.run()


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        colors = GradientColors()
        print(f"\n\n{colors.gradient_text('Cancelled by user')}\n")
        sys.exit(1)
    except Exception as e:
        colors = GradientColors()
        print(f"\n\n{colors.gradient_text(f'Error: {e}')}\n")
        import traceback
        traceback.print_exc()
        sys.exit(1)

"""Terminal output — gradient UI system. Battleship grey → asparagus green."""

import os
import sys
import time
from typing import List

# Enable ANSI true-color on Windows
if sys.platform == 'win32':
    try:
        import ctypes
        kernel32 = ctypes.windll.kernel32
        kernel32.SetConsoleMode(kernel32.GetStdHandle(-11), 7)
    except Exception:
        os.system('')

IS_TTY: bool = sys.stdout.isatty()


class G:
    """Gradient color engine — battleship grey (#878787) → asparagus green (#82A473)."""

    GREY  = (135, 135, 135)
    GREEN = (130, 164, 115)

    RESET = '\033[0m' if IS_TTY else ''
    BOLD  = '\033[1m' if IS_TTY else ''

    @staticmethod
    def _rgb(r: int, g: int, b: int) -> str:
        return f'\033[38;2;{r};{g};{b}m'

    @staticmethod
    def _rgb_bg(r: int, g: int, b: int) -> str:
        return f'\033[48;2;{r};{g};{b}m'

    @classmethod
    def _lerp(cls, c1, c2, t: float):
        t = max(0.0, min(1.0, t))
        return (
            int(c1[0] + (c2[0] - c1[0]) * t),
            int(c1[1] + (c2[1] - c1[1]) * t),
            int(c1[2] + (c2[2] - c1[2]) * t),
        )

    @classmethod
    def text(cls, s: str, reverse: bool = False) -> str:
        """Apply horizontal grey→green gradient to a string."""
        if not IS_TTY or not s:
            return s
        a, b = (cls.GREEN, cls.GREY) if reverse else (cls.GREY, cls.GREEN)
        n = len(s)
        out = ''
        for i, ch in enumerate(s):
            t = i / (n - 1) if n > 1 else 0.0
            out += cls._rgb(*cls._lerp(a, b, t)) + ch
        return out + cls.RESET

    @classmethod
    def logo_lines(cls, lines: List[str]) -> List[str]:
        """Apply vertical gradient (grey top → green bottom) to ASCII art lines."""
        if not IS_TTY:
            return lines
        n = len(lines)
        return [
            cls._rgb(*cls._lerp(cls.GREY, cls.GREEN, i / (n - 1) if n > 1 else 0.0))
            + line + cls.RESET
            for i, line in enumerate(lines)
        ]

    @classmethod
    def separator(cls, width: int = 60, reverse: bool = False) -> str:
        """Gradient ═ separator line."""
        if not IS_TTY:
            return '═' * width
        a, b = (cls.GREEN, cls.GREY) if reverse else (cls.GREY, cls.GREEN)
        out = ''
        for i in range(width):
            t = i / (width - 1) if width > 1 else 0.0
            out += cls._rgb(*cls._lerp(a, b, t)) + '═'
        return out + cls.RESET

    @classmethod
    def progress_bar(cls, current: int, total: int, width: int = 40) -> str:
        """Gradient filled progress bar with ░ for the unfilled portion."""
        pct = current / max(total, 1)
        filled = int(width * pct)
        bar = ''
        for i in range(width):
            if i < filled:
                t = i / max(width - 1, 1)
                bar += cls._rgb_bg(*cls._lerp(cls.GREY, cls.GREEN, t)) + ' '
            else:
                bar += cls._rgb(60, 60, 60) + '░'
        bar += cls.RESET
        pct_label = (
            f'{cls._rgb(*cls.GREEN)}{int(pct * 100)}%{cls.RESET}'
            if IS_TTY else f'{int(pct * 100)}%'
        )
        return f'  [{bar}] {pct_label}'

    @classmethod
    def color(cls, status: str) -> str:
        """ANSI color escape for a named status level."""
        if not IS_TTY:
            return ''
        return {
            'success': cls._rgb(*cls.GREEN),
            'warning': cls._rgb(255, 193, 7),
            'error':   cls._rgb(220, 53, 69),
            'info':    cls._rgb(*cls.GREY),
            'verify':  cls._rgb(0, 150, 255),
        }.get(status.lower(), cls.RESET)


# ─── ASCII art logo ───────────────────────────────────────────────────────────

_LOGO = [
    " █████╗ ██╗",
    "██╔══██╗██║",
    "███████║██║",
    "██╔══██║██║",
    "██║  ██║██║",
    "╚═╝  ╚═╝╚═╝",
]

# ─── Output helpers ───────────────────────────────────────────────────────────

_SYMBOLS = {
    'success': '✓',
    'error':   '✗',
    'warning': '⚠',
    'verify':  '🔎',
    'info':    '→',
}


def print_status(message: str, status: str = 'info', indent: int = 1) -> None:
    """Print a status line with a symbol prefix and gradient message."""
    pad = '  ' * indent
    sym = G.color(status) + _SYMBOLS.get(status, '→') + G.RESET
    print(f'{pad}{sym} {G.text(message)}')


def ok(msg: str) -> None:
    print_status(msg, 'success')


def info(msg: str) -> None:
    print_status(msg, 'info')


def warn(msg: str) -> None:
    print_status(msg, 'warning')


def error(msg: str) -> None:
    pad = '  '
    sym = G.color('error') + _SYMBOLS['error'] + G.RESET
    print(f'{pad}{sym} {G.text(msg)}', file=sys.stderr)


def step(title: str) -> None:
    """Gradient section header — double ═ separators with ▶ title."""
    width = 60
    print(f'\n{G.separator(width)}')
    print(f'  {G.text("▶ " + title, reverse=True)}')
    print(G.separator(width, reverse=True))


def header(title: str) -> None:
    """Alias for step() — major phase header."""
    step(title)


def tree_row(key: str, value: str, is_last: bool = False) -> None:
    """Print a tree-style key: value row with ├─ or └─ branch connector."""
    branch = '└─' if is_last else '├─'
    ic = G.color('info')
    sc = G.color('success')
    print(f'  {ic}{branch}{G.RESET} {G.text(key)}: {sc}{value}{G.RESET}')


def prompt(msg: str, default: str = '') -> str:
    """Gradient input prompt. Returns default if the user presses Enter."""
    hint = f' [{default}]' if default else ''
    label = G.text(f'{msg}{hint}:', reverse=True)
    try:
        val = input(f'  {label} ').strip()
        return val if val else default
    except (EOFError, KeyboardInterrupt):
        print()
        return default


def confirm(msg: str, default: bool = True) -> bool:
    """Gradient yes/no prompt. Returns default if the user presses Enter."""
    hint = 'Y/n' if default else 'y/N'
    label = G.text(f'{msg} [{hint}]:', reverse=True)
    try:
        val = input(f'  {label} ').strip().lower()
        if not val:
            return default
        return val in ('y', 'yes')
    except (EOFError, KeyboardInterrupt):
        print()
        return default


def pause(msg: str = 'Press Enter to exit...') -> None:
    """Gradient pause / exit prompt."""
    try:
        input(f'\n  {G.text(msg)}')
    except (EOFError, KeyboardInterrupt):
        print()


def show_banner(version: str, animated: bool = True) -> None:
    """
    Clear the screen and display the AI-Foundation logo.

    Animated mode: characters print one at a time with timing delays (interactive).
    Static mode:   instant print, no delays (non-interactive / --yes).
    """
    os.system('cls' if sys.platform == 'win32' else 'clear')
    print('\n\n')

    if animated and IS_TTY:
        for line in G.logo_lines(_LOGO):
            print('     ', end='')
            for ch in line:
                print(ch, end='', flush=True)
                time.sleep(0.002)
            print()
            time.sleep(0.05)

        time.sleep(0.3)
        subtitle = f'F O U N D A T I O N   v{version}'
        print('\n     ', end='')
        for ch in G.text(subtitle, reverse=True):
            print(ch, end='', flush=True)
            time.sleep(0.0025)
        print('\n')
        time.sleep(0.5)
    else:
        for line in G.logo_lines(_LOGO):
            print(f'     {line}')
        subtitle = f'F O U N D A T I O N   v{version}'
        print(f'\n     {G.text(subtitle, reverse=True)}\n')

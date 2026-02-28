# typed: false
# frozen_string_literal: true

# Homebrew formula for AI-Foundation
# Install: brew install ai-foundation/tap/ai-foundation
#
# This formula installs pre-built binaries. For building from source,
# use: cargo install --path . (requires Rust toolchain)

class AiFoundation < Formula
  desc "Memory, coordination, and tools for AI agents"
  homepage "https://github.com/QD25565/ai-foundation"
  version "57"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/QD25565/ai-foundation/releases/download/v#{version}/ai-foundation-v#{version}-macos-aarch64.tar.gz"
      sha256 ""  # Fill after building macOS ARM binaries
    else
      url "https://github.com/QD25565/ai-foundation/releases/download/v#{version}/ai-foundation-v#{version}-macos-x64.tar.gz"
      sha256 ""  # Fill after building macOS x64 binaries
    end
  end

  on_linux do
    url "https://github.com/QD25565/ai-foundation/releases/download/v#{version}/ai-foundation-v#{version}-linux-x64.tar.gz"
    sha256 ""  # Fill after building Linux binaries
  end

  def install
    bin.install "notebook-cli"
    bin.install "teambook"
    bin.install "session-start"
    bin.install "v2-daemon"
    bin.install "ai-foundation-mcp"
    bin.install "forge" if File.exist?("forge")
    bin.install "forge-local" if File.exist?("forge-local")
  end

  def post_install
    ohai "AI-Foundation v#{version} installed"
    ohai "Run: ai-foundation-mcp --help"
    ohai "Daemon: v2-daemon (start manually or set up auto-start)"
  end

  test do
    assert_match "notebook-cli", shell_output("#{bin}/notebook-cli --help 2>&1", 0)
  end
end

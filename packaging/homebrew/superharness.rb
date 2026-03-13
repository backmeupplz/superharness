# typed: false
# frozen_string_literal: true

# This formula is intended to be kept in a personal tap:
#   https://github.com/backmeupplz/homebrew-superharness
#
# Install with:
#   brew tap backmeupplz/superharness
#   brew install superharness
class Superharness < Formula
  desc "Autonomous multi-agent orchestrator for AI coding agents via tmux"
  homepage "https://github.com/backmeupplz/superharness"
  version "0.2.0"
  license "MIT"

  # Pre-built binaries — one URL block per platform/architecture.
  # SHA256 values are updated automatically by the Homebrew workflow on release.
  on_macos do
    on_intel do
      url "https://github.com/backmeupplz/superharness/releases/download/v#{version}/superharness-v#{version}-x86_64-apple-darwin"
      sha256 "PLACEHOLDER_MACOS_AMD64"
    end

    on_arm do
      url "https://github.com/backmeupplz/superharness/releases/download/v#{version}/superharness-v#{version}-aarch64-apple-darwin"
      sha256 "PLACEHOLDER_MACOS_ARM64"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/backmeupplz/superharness/releases/download/v#{version}/superharness-v#{version}-x86_64-unknown-linux-musl"
      sha256 "PLACEHOLDER_LINUX_AMD64"
    end

    on_arm do
      url "https://github.com/backmeupplz/superharness/releases/download/v#{version}/superharness-v#{version}-aarch64-unknown-linux-musl"
      sha256 "PLACEHOLDER_LINUX_ARM64"
    end
  end

  # No build step needed — we ship pre-built binaries.
  def install
    # The downloaded file is named after the asset URL; rename it on install.
    binary = Dir["superharness-v#{version}-*"].first
    bin.install binary => "superharness"
  end

  test do
    assert_match "superharness", shell_output("#{bin}/superharness --help")
  end
end

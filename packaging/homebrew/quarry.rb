class Quarry < Formula
  desc "Fast SQL client and TUI for PostgreSQL, MySQL / MariaDB and SQLite"
  homepage "https://quarry.asmechanics.com"
  url "https://github.com/mah3uz/quarry/archive/refs/tags/v0.2.0.tar.gz"
  sha256 "9125110845948fb586e13d3f9f28291bd979c1f6a33771788bbee1917cc1e0e3"
  license "MIT"
  head "https://github.com/mah3uz/quarry.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
    generate_completions_from_executable(bin/"quarry", "--completions")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/quarry --version")
    assert_equal "n\n1\n", shell_output("#{bin}/quarry :memory: -e 'select 1 as n' --format csv")
  end
end

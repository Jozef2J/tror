class Trorpad < Formula
  desc "Fast, compact code editor with SFTP/FTP, SSH console and database tools"
  homepage "https://github.com/Jozef2J/tror"
  url "https://github.com/Jozef2J/tror/releases/download/v0.30.2/trorpad-linux-x86_64"
  version "0.30.2"
  sha256 "f0d4eb91c2f9060b9e0e1f5aa225c2ce70437da24ab6f1dc9c0b03b7226e55d5"
  license any_of: ["MIT", "Apache-2.0"]

  depends_on arch: :x86_64
  depends_on :linux

  def install
    bin.install "trorpad-linux-x86_64" => "trorpad"
  end

  test do
    assert_path_exists bin/"trorpad"
  end
end

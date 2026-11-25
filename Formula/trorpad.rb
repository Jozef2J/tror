class Trorpad < Formula
  desc "Fast, compact code editor with SFTP/FTP, SSH console and database tools"
  homepage "https://github.com/Jozef2J/tror"
  url "https://github.com/Jozef2J/tror/releases/download/v0.30.6/trorpad-linux-x86_64"
  version "0.30.6"
  sha256 "06caa9f47ad975a2df50eab5777a94af4dcb8a0222829bc5461e4fca7b56f3f1"
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

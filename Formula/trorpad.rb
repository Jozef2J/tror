class Trorpad < Formula
  desc "Fast, compact code editor with SFTP/FTP, SSH console and database tools"
  homepage "https://github.com/Jozef2J/tror"
  url "https://github.com/Jozef2J/tror/releases/download/v0.30.5/trorpad-linux-x86_64"
  version "0.30.5"
  sha256 "814eb52a39ff89cb40f975f5357992fa15850906436d2dc48f137c91581f75db"
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

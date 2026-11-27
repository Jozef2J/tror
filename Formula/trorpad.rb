class Trorpad < Formula
  desc "Fast, compact code editor with SFTP/FTP, SSH console and database tools"
  homepage "https://github.com/Jozef2J/tror"
  url "https://github.com/Jozef2J/tror/releases/download/v0.31.0/trorpad-linux-x86_64"
  version "0.31.0"
  sha256 "c84ae902406cc91a413645485935b7f7e88b706767e90a1abb5eb8dace698da7"
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

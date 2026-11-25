class Trorpad < Formula
  desc "Fast, compact code editor with SFTP/FTP, SSH console and database tools"
  homepage "https://github.com/Jozef2J/tror"
  url "https://github.com/Jozef2J/tror/releases/download/v0.30.3/trorpad-linux-x86_64"
  version "0.30.3"
  sha256 "7e85d6bc97abe441f66ad24b9c6ec0831975b5e078b6240336039f6cc77d2449"
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

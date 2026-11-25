cask "trorpad" do
  version "0.30.2"
  sha256 "3cf94d740521df4a27fb15ec52221ecfb2dc4ef40621161b34b95ed018efc53b"

  url "https://github.com/Jozef2J/tror/releases/download/v#{version}/TrorPad-macos.zip"
  name "TrorPad"
  desc "Fast, compact code editor with SFTP/FTP, SSH console and database tools"
  homepage "https://github.com/Jozef2J/tror"

  app "TrorPad.app"

  zap trash: "~/Library/Application Support/Tror"

  caveats <<~EOS
    TrorPad is not signed by Apple yet. If macOS refuses to open it, run:
      xattr -dr com.apple.quarantine "#{appdir}/TrorPad.app"
  EOS
end

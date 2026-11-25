cask "trorpad" do
  version "0.30.6"
  sha256 "e549fb2f8bf2c7cb9779fb06e4997dca19e4a183ce5f5d9875cb9e7e94bfb2c6"

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

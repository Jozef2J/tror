cask "trorpad" do
  version "0.30.7"
  sha256 "3c66529e1f0f1b07a52bb472b066fcb5e764dc8ca2b7b93da74f5f55b795dbb0"

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

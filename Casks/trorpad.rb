cask "trorpad" do
  version "0.31.0"
  sha256 "838a7c3abaac412977bce48368149dfdf6b27f6892ff1edeebcfbf5e6cb2f62d"

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

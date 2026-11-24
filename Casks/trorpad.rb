cask "trorpad" do
  version "0.30.0"
  sha256 "4512f75799c3cd73e08a8227b47a84a6d69192a0a5693f813768ce0071dd6c35"

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

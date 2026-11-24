import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
REPO = "https://github.com/Jozef2J/tror"
DESCRIPTION = "Fast, compact code editor with SFTP/FTP, SSH console and database tools"


def sums(path):
    out = {}
    for line in pathlib.Path(path).read_text().splitlines():
        parts = line.split()
        if len(parts) == 2:
            out[parts[1].lstrip("*")] = parts[0]
    return out


def scoop(version, hashes):
    return f"""{{
    "version": "{version}",
    "description": "{DESCRIPTION}",
    "homepage": "{REPO}",
    "license": "MIT|Apache-2.0",
    "architecture": {{
        "64bit": {{
            "url": "{REPO}/releases/download/v{version}/trorpad.exe",
            "hash": "{hashes['trorpad.exe']}"
        }}
    }},
    "bin": "trorpad.exe",
    "shortcuts": [
        [
            "trorpad.exe",
            "TrorPad"
        ]
    ],
    "checkver": "github",
    "autoupdate": {{
        "architecture": {{
            "64bit": {{
                "url": "{REPO}/releases/download/v$version/trorpad.exe"
            }}
        }},
        "hash": {{
            "url": "$baseurl/SHA256SUMS"
        }}
    }}
}}
"""


def formula(version, hashes):
    return f"""class Trorpad < Formula
  desc "{DESCRIPTION}"
  homepage "{REPO}"
  url "{REPO}/releases/download/v{version}/trorpad-linux-x86_64"
  version "{version}"
  sha256 "{hashes['trorpad-linux-x86_64']}"
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
"""


def cask(version, sha):
    return f"""cask "trorpad" do
  version "{version}"
  sha256 "{sha}"

  url "{REPO}/releases/download/v#{{version}}/TrorPad-macos.zip"
  name "TrorPad"
  desc "{DESCRIPTION}"
  homepage "{REPO}"

  app "TrorPad.app"

  zap trash: "~/Library/Application Support/Tror"

  caveats <<~EOS
    TrorPad is not signed by Apple yet. If macOS refuses to open it, run:
      xattr -dr com.apple.quarantine "#{{appdir}}/TrorPad.app"
  EOS
end
"""


def winget(version, hashes):
    ident = "Jozef2J.TrorPad"
    head = "# yaml-language-server: $schema=https://aka.ms/winget-manifest.{kind}.1.9.0.schema.json\n\n"
    common = f"PackageIdentifier: {ident}\nPackageVersion: {version}\n"
    files = {
        f"{ident}.yaml": head.format(kind="version")
        + common
        + "DefaultLocale: en-US\nManifestType: version\nManifestVersion: 1.9.0\n",
        f"{ident}.installer.yaml": head.format(kind="installer")
        + common
        + f"""InstallerType: inno
Scope: machine
UpgradeBehavior: install
ReleaseDate: RELEASE_DATE
Installers:
- Architecture: x64
  InstallerUrl: {REPO}/releases/download/v{version}/trorpad-setup.exe
  InstallerSha256: {hashes['trorpad-setup.exe'].upper()}
ManifestType: installer
ManifestVersion: 1.9.0
""",
        f"{ident}.locale.en-US.yaml": head.format(kind="defaultLocale")
        + common
        + f"""PackageLocale: en-US
Publisher: Jozef2J
PublisherUrl: https://github.com/Jozef2J
PublisherSupportUrl: {REPO}/issues
PrivacyUrl: {REPO}/blob/main/PRIVACY.md
PackageName: TrorPad
PackageUrl: {REPO}
License: MIT OR Apache-2.0
LicenseUrl: {REPO}/blob/main/LICENSE-MIT
ShortDescription: {DESCRIPTION}
Description: A Notepad++-style editor written in Rust with an SFTP/FTP/FTPS file manager, SSH console, MySQL/PostgreSQL/SQLite tools and WebAssembly plugins.
Moniker: trorpad
Tags:
- editor
- sftp
- ftp
- ssh
- sql
ReleaseNotesUrl: {REPO}/releases/tag/v{version}
ManifestType: defaultLocale
ManifestVersion: 1.9.0
""",
    }
    return files


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    print(path.relative_to(ROOT))


def main():
    kind, version = sys.argv[1], sys.argv[2]
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        sys.exit(f"bad version {version}")
    if kind == "release":
        hashes = sums(sys.argv[3])
        date = sys.argv[4] if len(sys.argv) > 4 else ""
        write(ROOT / "bucket" / "trorpad.json", scoop(version, hashes))
        write(ROOT / "Formula" / "trorpad.rb", formula(version, hashes))
        target = ROOT / "packaging" / "winget"
        for old in target.glob("*.yaml"):
            old.unlink()
        for name, text in winget(version, hashes).items():
            if date:
                text = text.replace("RELEASE_DATE", date)
            else:
                text = text.replace("ReleaseDate: RELEASE_DATE\n", "")
            write(target / name, text)
    elif kind == "mac":
        write(ROOT / "Casks" / "trorpad.rb", cask(version, sys.argv[3]))
    else:
        sys.exit(f"unknown kind {kind}")


main()

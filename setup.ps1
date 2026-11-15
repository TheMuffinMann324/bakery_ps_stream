# Installs the Windows x64 build dependencies from vcpkg.json.
$ErrorActionPreference = 'Stop'

function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program exited with code $LASTEXITCODE"
    }
}

function Apply-Patch {
    param([string]$Repository, [string]$Patch, [string[]]$Options = @())
    & git -C $Repository apply --reverse --check @Options $Patch 2>$null
    if ($LASTEXITCODE -eq 0) { return }
    Invoke-Checked git (@('-C', $Repository, 'apply', '--check') + $Options + $Patch)
    Invoke-Checked git (@('-C', $Repository, 'apply') + $Options + $Patch)
}

Push-Location -LiteralPath $PSScriptRoot
try {
    $vs = $env:VS_INSTALL_DIR
    if (-not $vs) {
        $vswhere = Join-Path "${env:ProgramFiles(x86)}" 'Microsoft Visual Studio\Installer\vswhere.exe'
        $vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 Microsoft.VisualStudio.Component.VC.CMake.Project -property installationPath
        if ($LASTEXITCODE -ne 0 -or -not $vs) {
            throw 'Install Visual Studio with Desktop development with C++, including CMake and vcpkg.'
        }
    }
    $vcpkg = Join-Path $vs 'VC\vcpkg\vcpkg.exe'
    if (-not (Test-Path -LiteralPath $vcpkg -PathType Leaf)) {
        throw 'Install the vcpkg component in Visual Studio or set VS_INSTALL_DIR to the correct installation.'
    }

    $chiaki = 'third_party/chiaki-ng'
    if (Test-Path -LiteralPath '.git') {
        Invoke-Checked git @('submodule', 'update', '--init', $chiaki)
        Invoke-Checked git @('-C', $chiaki, 'submodule', 'update', '--init', 'third-party/nanopb', 'third-party/jerasure', 'third-party/gf-complete')
    }
    if (-not (Test-Path -LiteralPath "$chiaki/third-party/nanopb/CMakeLists.txt" -PathType Leaf)) {
        throw 'Clone with Git or unpack the complete release source archive.'
    }
    Apply-Patch $chiaki 'scripts/windows-vc/gf-complete.patch' @('--ignore-whitespace', '--directory=third-party/gf-complete/')
    Apply-Patch $chiaki '../../native/patches/chiaki-msvc.patch'

    $python = if ($env:PS_PYTHON) { $env:PS_PYTHON } else { 'python' }
    if (-not (Test-Path -LiteralPath '.venv/Scripts/python.exe' -PathType Leaf)) {
        Invoke-Checked $python @('-m', 'venv', '.venv')
    }
    Invoke-Checked '.venv/Scripts/python.exe' @('-m', 'pip', 'install', '-r', 'requirements-build.txt')
    Invoke-Checked $vcpkg @('install', '--triplet', 'x64-windows', '--x-install-root=vcpkg_installed')
    Write-Host 'Build and run with cargo run --release --locked'
} finally {
    Pop-Location
}

$ErrorActionPreference = 'Stop'

$extractor = Join-Path $env:ProgramFiles '7-Zip\7z.exe'
if (-not (Test-Path -LiteralPath $extractor)) {
    throw "Install 7-Zip to prepare the Windows build resources."
}

$destination = Join-Path $PSScriptRoot '..\src-tauri\resources\archive-tool'
$scratch = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $scratch | Out-Null
try {
    $archive = Join-Path $scratch '7zip-x64.exe'
    Invoke-WebRequest -Uri 'https://github.com/ip7z/7zip/releases/download/26.06/7z2606-x64.exe' -OutFile $archive
    $digest = (Get-FileHash -Algorithm SHA256 -LiteralPath $archive).Hash
    if ($digest -ne '9b697f65ecc2d9b0152f5ee96d72dde522a7827690349d9307935dacf40f42ec') {
        throw "The 7-Zip download failed its integrity check."
    }
    New-Item -ItemType Directory -Force -Path $destination | Out-Null
    & $extractor e $archive "-o$destination" -y '7z.exe' '7z.dll' 'License.txt' 'readme.txt'
    if ($LASTEXITCODE -ne 0) { throw "Could not extract the Windows archive tool." }
    foreach ($file in @('7z.exe', '7z.dll', 'License.txt', 'readme.txt')) {
        if (-not (Test-Path -LiteralPath (Join-Path $destination $file) -PathType Leaf)) {
            throw "Missing 7-Zip resource $file."
        }
    }
    '7-Zip 26.06 source code and releases: https://github.com/ip7z/7zip/releases/tag/26.06' |
        Set-Content -LiteralPath (Join-Path $destination 'SOURCE.txt') -Encoding utf8
} finally {
    Remove-Item -LiteralPath $scratch -Recurse -Force
}

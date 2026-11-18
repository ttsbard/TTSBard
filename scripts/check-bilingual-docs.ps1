[CmdletBinding()]
param([string]$Root)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
# The default must be resolved in the body: in Windows PowerShell 5.1
# (-NoProfile -File) $PSScriptRoot is empty inside a param default
# expression, while in the script body it is set in every host (both
# powershell.exe and pwsh). An explicit -Root keeps working.
if ([string]::IsNullOrWhiteSpace($Root)) {
    $Root = Join-Path $PSScriptRoot '..'
}
$Root = [IO.Path]::GetFullPath($Root)
$failures = [Collections.Generic.List[string]]::new()
$required = @(
    'README.md', 'faq.md', 'ttsbard-overview.md',
    'user/README.md', 'user/guide.md', 'user/editor.md', 'user/tts.md',
    'user/virtual-audio-cable.md', 'user/audio-devices.md', 'user/audio-effects.md',
    'user/dsp-settings.md', 'user/ai-text.md', 'user/language-packs.md',
    'user/incoming.md', 'user/input-server.md', 'user/ocr.md', 'user/ruaccent.md',
    'user/silero-settings.md', 'user/soundpanel.md',
    'integrations/README.md', 'integrations/webview.md', 'integrations/sse.md',
    'integrations/twitch.md', 'integrations/vrchat.md',
    'integrations/vtube-studio.md', 'integrations/lunatranslator/README.md'
)

$allPages = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
foreach ($page in $required) { $null = $allPages.Add($page) }
foreach ($locale in @('en', 'ru')) {
    $directory = Join-Path $Root "docs/$locale"
    if (-not (Test-Path -LiteralPath $directory)) {
        $failures.Add("Missing language directory: docs/$locale")
        continue
    }
    foreach ($file in Get-ChildItem -LiteralPath $directory -Filter '*.md' -Recurse -File) {
        $relative = $file.FullName.Substring($directory.TrimEnd('\', '/').Length + 1).Replace('\', '/')
        $null = $allPages.Add($relative)
    }
}

foreach ($page in $allPages) {
    foreach ($locale in @('en', 'ru')) {
        $path = Join-Path $Root "docs/$locale/$page"
        if (-not (Test-Path -LiteralPath $path)) {
            $failures.Add("Missing language pair: docs/$locale/$page")
            continue
        }
        $content = Get-Content -LiteralPath $path -Raw -Encoding UTF8
        # Exclude code and navigation before checking that the page is real content.
        $prose = [regex]::Replace($content, '(?ms)^```[^\r\n]*\r?\n.*?^```\s*$', '')
        $prose = [regex]::Replace($prose, '\[[^\]]*\]\([^)]+\)', '')
        if ($prose.Length -lt 250) {
            $failures.Add("Content is too short or a redirect: docs/$locale/$page")
        }
        if ($locale -eq 'en' -and $content -match '(?m)^#{1,6}\s+[^\r\n]*[\u0400-\u04ff]') {
            $failures.Add("Untranslated English heading: docs/en/$page")
        }
        $otherLocale = if ($locale -eq 'en') { 'ru' } else { 'en' }
        $other = [IO.Path]::GetFullPath((Join-Path $Root "docs/$otherLocale/$page"))
        $firstLine = ($content -split '\r?\n')[0]
        $switchMatches = [regex]::Matches($firstLine, '\[[^\]]+\]\((?<target>[^)]+)\)')
        $switchFound = $false
        foreach ($match in $switchMatches) {
            $target = [IO.Path]::GetFullPath((Join-Path (Split-Path $path) $match.Groups['target'].Value))
            if ($target -eq $other) { $switchFound = $true }
        }
        if (-not $switchFound) {
            $failures.Add("Missing language switch to matching page: docs/$locale/$page")
        }
    }
}
if ($failures.Count) {
    foreach ($failure in $failures) { Write-Output "ERROR: $failure" }
    exit 1
}
Write-Output "Bilingual documentation passed: $($allPages.Count) EN/RU page pairs."

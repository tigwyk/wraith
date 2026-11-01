# Wraith shell integration for PowerShell: reports the working directory (OSC 7) and prompt marks (OSC 133).
if ($env:TERM_PROGRAM -ne 'wraith' -or $global:__WraithIntegrated) { return }
$global:__WraithIntegrated = $true
$global:__WraithOrigPrompt = $function:prompt
$global:__WraithStarted = $false

function global:prompt {
    $ok = $?
    $esc = [char]27
    $out = ''
    if ($global:__WraithStarted) { $out += "$esc]133;D;$([int](-not $ok))$esc\" }
    $global:__WraithStarted = $true
    if ($PWD.Provider.Name -eq 'FileSystem') {
        $p = ($PWD.ProviderPath -replace '\\', '/')
        $out += "$esc]7;file://$($env:COMPUTERNAME)/$([uri]::EscapeUriString($p))$esc\"
    }
    $out += "$esc]133;A$esc\"
    $text = & $global:__WraithOrigPrompt
    "$out$text$esc]133;B$esc\"
}

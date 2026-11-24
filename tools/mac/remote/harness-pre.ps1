# SPDX-License-Identifier: GPL-3.0-or-later
# pc harness capture, before the job: read-only notes for the report (research 16 README section
# 8.7). AMD's Instant Replay can hold a video-encoder session beside the capture's, so its
# registry hints are recorded next to the verdict; nothing is ever written and AMD's settings are
# never toggled (CB:1066 marks both values unverified). Also the pinned FFmpeg build's asset name,
# which the Mac compares with the bootstrap's pin. Never refuses: it changes nothing, so the busy
# state is the job's own check to make.

function Get-RegistryHint([string]$Key, [string]$Name) {
    # An integer, 'absent' or 'unreadable'.
    try {
        $item = Get-ItemProperty -LiteralPath $Key -Name $Name -ErrorAction Stop
        $value = $item.$Name
        if ($value -is [int] -or $value -is [long]) { return [string]$value }
        return 'unreadable'
    } catch [Management.Automation.ItemNotFoundException] {
        return 'absent'
    } catch [Management.Automation.PSArgumentException] {
        return 'absent'
    } catch {
        return 'unreadable'
    }
}

# TODO(verify): the value names of Adrenalin's Instant Replay settings (CB:1066).
Send-KV 'ir_replay' (Get-RegistryHint 'HKLM:\SOFTWARE\AMD\DVR' 'InstantReplayEnabled')
# HKCU is this SSH session's account: today the owner's (M1:172).
Send-KV 'ir_dvr' (Get-RegistryHint 'HKCU:\Software\AMD\DVR' 'DvrEnabled')

$asset = 'unreadable'
$pin = Read-JsonFile (Join-Path $Kairo.Tools 'ffmpeg\KAIRO-PIN.json')
if ($null -ne $pin) {
    $value = [string](Get-Field $pin 'asset')
    if ($value -cmatch '^[A-Za-z0-9._-]{1,120}\z') { $asset = $value }
}
Send-KV 'ffmpeg_asset' $asset
Exit-Pc 0

# XBM -> ASCII art + MSB-first byte array (firmware blit format)
$ErrorActionPreference = 'Stop'
foreach ($f in (Get-ChildItem 'C:\Users\lemon\keypoint-rmk\tools\xbm\*.xbm')) {
    $txt = [IO.File]::ReadAllText($f.FullName)
    $w = [int]([regex]::Match($txt, 'width\s+(\d+)').Groups[1].Value)
    $h = [int]([regex]::Match($txt, 'height\s+(\d+)').Groups[1].Value)
    $m = [regex]::Match($txt, 'bits\w*\s*\[\s*\]\s*=\s*\{(.*?)\};', 'Singleline')
    $body = [regex]::Replace($m.Groups[1].Value, '/\*.*?\*/', '', 'Singleline')
    $bytes = @()
    foreach ($tok in ($body -split ',')) {
        $t = $tok.Trim()
        if ($t -match '^0[xX][0-9a-fA-F]+$') { $bytes += [Convert]::ToInt32($t.Substring(2),16) }
        elseif ($t -match '^\d+$') { $bytes += [Convert]::ToInt32($t,10) }
    }
    $rowbytes = [Math]::Ceiling($w / 8)
    Write-Output ("== " + $f.Name + "  " + $w + "x" + $h)
    # art: XBM LSB-first within byte, rows padded to byte
    for ($y=0; $y -lt $h; $y++) {
        $line=''
        for ($x=0; $x -lt $w; $x++) {
            $b = $bytes[$y*$rowbytes + ($x -shr 3)]
            if ($b -band (1 -shl ($x -band 7))) { $line+='#' } else { $line+='.' }
        }
        Write-Output $line
    }
    # re-emit MSB-first rows for firmware
    $out = @()
    for ($y=0; $y -lt $h; $y++) {
        $row = New-Object int[] $rowbytes
        for ($x=0; $x -lt $w; $x++) {
            $b = $bytes[$y*$rowbytes + ($x -shr 3)]
            if ($b -band (1 -shl ($x -band 7))) { $row[$x -shr 3] = $row[$x -shr 3] -bor (0x80 -shr ($x -band 7)) }
        }
        $out += $row
    }
    Write-Output (("MSB[{0}] = {{ {1} }}" -f $out.Count, (($out | ForEach-Object { '0x{0:X2}' -f $_ }) -join ', ')))
    Write-Output ''
}

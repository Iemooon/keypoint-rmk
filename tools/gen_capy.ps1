# Convert ZMK capy_XX.c (120x72 landscape, 1bpp, 0=lit) into a portrait
# 72x120 Rust array (1=lit) for the RMK renderers.
$ErrorActionPreference = 'Stop'
$src = 'C:\Users\lemon\Documents\GitHub\keypoint-zmk-dongle\config\boards\shields\lpm_view\widgets\picture'
$out = New-Object System.Text.StringBuilder
[void]$out.AppendLine('//! Capybara animation frames, generated from the ZMK project''s')
[void]$out.AppendLine('//! `widgets/picture/capy_01..09.c` (LVGL INDEXED_1BIT, 120x72 landscape, 0 = lit).')
[void]$out.AppendLine('//! Here: portrait 70x120 (white edge columns 0 and 71 trimmed),')
[void]$out.AppendLine('//! row-major MSB-first, 9 bytes/row, 1 = lit.')
[void]$out.AppendLine('//! Mapping (ZMK landscape -> our portrait canvas): x'' = 71 - v, y'' = 20 + u.')
[void]$out.AppendLine('//! DO NOT EDIT BY HAND - regenerate with tools/gen_capy.ps1.')
[void]$out.AppendLine('')
[void]$out.AppendLine('pub const CAPY_W: usize = 70;')
[void]$out.AppendLine('pub const CAPY_H: usize = 120;')
[void]$out.AppendLine('pub const CAPY_FRAMES: [[u8; 1080]; 9] = [')

foreach ($n in 1..9) {
    $file = Join-Path $src ("capy_{0:d2}.c" -f $n)
    $txt = [IO.File]::ReadAllText($file)
    $m = [regex]::Match($txt, 'capy_\d+_map\[\]\s*=\s*\{(.*?)\};', 'Singleline')
    $body = [regex]::Replace($m.Groups[1].Value, '(?s)#if.*?#else', '')
    $body = [regex]::Replace($body, '(?m)^\s*#endif\s*$', '')
    $body = [regex]::Replace($body, '/\*.*?\*/', '', 'Singleline')
    $bytes = @()
    foreach ($tok in ($body -split ',')) {
        $t = $tok.Trim()
        if ($t -match '^0[xX][0-9a-fA-F]+$') { $bytes += [Convert]::ToInt32($t.Substring(2),16) }
    }
    # first 8 bytes = palette, then 72 rows x 15 bytes
    if ($bytes.Count -ne (8 + 1080)) { throw ("frame " + $n + " unexpected " + $bytes.Count + " bytes") }
    $lum = $bytes[8..($bytes.Count-1)]
    $art = New-Object int[] (120*9)
    for ($r=0; $r -lt 120; $r++) {          # portrait row = u (landscape col)
        for ($c=1; $c -lt 71; $c++) {      # drop white cols 0 and 71 -> 70px
            $v = 71 - $c
            $lb = $lum[$v*15 + ($r -shr 3)]
            $bit = ($lb -shr (7 - ($r -band 7))) -band 1
            $o = $c - 1
            if ($bit -eq 0) { $art[$r*9 + ($o -shr 3)] = $art[$r*9 + ($o -shr 3)] -bor (0x80 -shr ($o -band 7)) }
        }
    }
    [void]$out.AppendLine('    [')
    for ($r=0; $r -lt 120; $r+=6) {
        $chunk = @()
        for ($k=$r; $k -lt [Math]::Min($r+6,120); $k++) {
            $row = $art[($k*9)..($k*9+8)]
            $chunk += ('        ' + (($row | ForEach-Object { '0x{0:X2}' -f $_ }) -join ', ') + ',')
        }
        [void]$out.AppendLine(($chunk -join "`n"))
    }
    [void]$out.AppendLine('    ], // capy ' + ('{0:d2}' -f $n))
}
[void]$out.AppendLine('];')
[IO.File]::WriteAllText('C:\Users\lemon\keypoint-rmk\src\capy_art.rs', $out.ToString())
Write-Output 'capy_art.rs written'

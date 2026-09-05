# Preview of the Keypoint screen UI (renderers.rs rev.3) via System.Drawing.
Add-Type -AssemblyName System.Drawing
$S = 3
$W = 72 * $S; $H = 144 * $S; $GAP = 90
$Bt = @(0x3E,0x00,0x67,0x00,0xE3,0x80,0xE9,0x80,0x8C,0x80,0xC9,0x80,0xE3,0x80,0xE3,0x80,
        0xC9,0x80,0x8C,0x80,0xE9,0x80,0xE3,0x80,0x67,0x00,0x3E,0x00)
$Usb = @(0x08,0x00,0x1C,0x00,0x0A,0x00,0xC8,0xC0,0x48,0x40,0x2A,0x00,0xC8,0x00,0x08,0x00,
         0x08,0x00,0x08,0x00,0x14,0x00,0x20,0x80,0x20,0x80,0x14,0x00)
$TopY=8; $NumY=10; $BarX=28; $NumX=53; $LayerY=118
$white=[System.Drawing.Color]::FromArgb(235,238,235)
$black=[System.Drawing.Color]::FromArgb(12,14,12)
$bg=[System.Drawing.Color]::FromArgb(40,42,40)
$totalW = $W*3 + $GAP*2; $totalH = $H + 60
$bmp = New-Object System.Drawing.Bitmap($totalW,$totalH)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.Clear($bg)
$f13 = New-Object System.Drawing.Font("Consolas", 19, [System.Drawing.GraphicsUnit]::Pixel)
$f18 = New-Object System.Drawing.Font("Consolas", 28, [System.Drawing.GraphicsUnit]::Pixel)
$bW = New-Object System.Drawing.SolidBrush($white)
$bK = New-Object System.Drawing.SolidBrush($black)

function PR([int]$ox,[int]$x,[int]$y,[int]$w,[int]$h,$b){
  $g.FillRectangle($b, ($ox + $x*$S), (40 + $y*$S), ($w*$S), ($h*$S))
}

function Panel([int]$ox,[bool]$usbOn,[int]$profile,[bool]$adv,[int]$level,[string]$layer){
  $g.FillRectangle($bK, $ox, 40, $W, $H)
  if ($usbOn) {
    $icon = $Usb
    for($row=0; $row -lt 14; $row++){
      $word = ($icon[$row*2] -shl 8) -bor $icon[$row*2+1]
      for($col=0; $col -lt 9; $col++){
        if ($word -band (0x8000 -shr $col)) { PR $ox (1+$col) ($TopY+$row) 1 1 $bW }
      }
    }
  } else {
    for($row=0; $row -lt 14; $row++){
      $word = ($Bt[$row*2] -shl 8) -bor $Bt[$row*2+1]
      for($col=0; $col -lt 9; $col++){
        if ($word -band (0x8000 -shr $col)) { PR $ox (1+$col) ($TopY+$row) 1 1 $bW }
      }
    }
    $digit = if ($adv) { "-" } else { "$profile" }
    $g.DrawString($digit, $f13, $bW, ($ox + 12*$S), (40 + 10*$S))
  }
  PR $ox $BarX        ($TopY+1)  22 11 $bW    # body
  PR $ox ($BarX+1)    ($TopY+2)  20  9 $bK    # inside carved out
  if ($level -ge 0) {
    $fw = [math]::Max(1,[math]::Min(18,[int]($level*18/100)))
    PR $ox ($BarX+2)  ($TopY+3)  $fw 7 $bW    # charge
    $num = "$level"
  } else { $num = "--" }
  PR $ox ($BarX+22)   ($TopY+4)  2 5 $bW      # cap
  $g.DrawString($num, $f13, $bW, ($ox + $NumX*$S), (40 + 10*$S))
  $lw = $layer.Length * 9
  $g.DrawString($layer, $f18, $bW, ($ox + [int]((72-$lw)/2)*$S), (40 + $LayerY*$S))
  $g.DrawRectangle((New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(90,95,90))), ($ox-1), 39, ($W+2), ($H+2))
}

Panel 0 $false 1 $false 68 "RAISE"
Panel ($W+$GAP) $false 1 $true 45 "LOWER"
Panel (2*($W+$GAP)) $true 0 $false 100 "BASE"
$labels = @("Bluetooth linked (slot 1)","Pairing / advertising","Wired")
for($i=0;$i -lt 3;$i++){ $g.DrawString($labels[$i], $f13, $bW, ($i*($W+$GAP)+6), 8) }

$out = "C:\Users\lemon\keypoint-rmk\screen-preview.png"
$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output "saved $out"

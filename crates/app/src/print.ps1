# Shows the print dialog of Windows, then prints the pages GodDoc draws into the folder
# GODDOC_PRINT_DIR as 1.png, 2.png, and so on. Says on its output: 'cancel', or
# 'pages <first> <last>' then 'done', or 'error <why>'.
# It is given to PowerShell as one argument: no double quotes in here.
$ErrorActionPreference = 'Stop'
try {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing
    $count = [int]$env:GODDOC_PRINT_PAGES
    $doc = New-Object System.Drawing.Printing.PrintDocument
    $doc.DocumentName = $env:GODDOC_PRINT_NAME
    # No small window saying which page is being printed.
    $doc.PrintController = New-Object System.Drawing.Printing.StandardPrintController
    $settings = $doc.PrinterSettings
    $settings.MinimumPage = 1
    $settings.MaximumPage = $count
    $settings.FromPage = 1
    $settings.ToPage = $count
    $dialog = New-Object System.Windows.Forms.PrintDialog
    $dialog.Document = $doc
    $dialog.AllowSomePages = $true
    $dialog.UseEXDialog = $true
    # An owner that stays on top: the dialog must not open behind GodDoc.
    $owner = New-Object System.Windows.Forms.Form
    $owner.TopMost = $true
    $owner.ShowInTaskbar = $false
    $owner.Opacity = 0
    $owner.StartPosition = 'CenterScreen'
    $owner.Show()
    $answer = $dialog.ShowDialog($owner)
    $owner.Close()
    if ($answer -ne 'OK') {
        [Console]::Out.WriteLine('cancel')
        exit 0
    }
    $global:page = 1
    $global:last = $count
    if ($settings.PrintRange -eq 'SomePages') {
        $global:page = $settings.FromPage
        $global:last = $settings.ToPage
    }
    [Console]::Out.WriteLine('pages ' + $global:page + ' ' + $global:last)
    [Console]::Out.Flush()

    # Before each page: wait for GodDoc to have drawn it, and turn the paper as it is turned.
    $doc.add_QueryPageSettings({
        param($sender, $e)
        $path = Join-Path $env:GODDOC_PRINT_DIR ([string]$global:page + '.png')
        $limit = (Get-Date).AddMinutes(5)
        while (-not (Test-Path -LiteralPath $path)) {
            if ((Get-Date) -gt $limit) { throw 'the page was not drawn in time' }
            Start-Sleep -Milliseconds 30
        }
        $bytes = [System.IO.File]::ReadAllBytes($path)
        [System.IO.File]::Delete($path)
        $stream = New-Object System.IO.MemoryStream (, $bytes)
        $global:image = [System.Drawing.Image]::FromStream($stream)
        $e.PageSettings.Landscape = $global:image.Width -gt $global:image.Height
    })
    # The page, as large as the paper lets it be, in its middle.
    $doc.add_PrintPage({
        param($sender, $e)
        $area = $e.Graphics.VisibleClipBounds
        $scale = [Math]::Min($area.Width / $global:image.Width, $area.Height / $global:image.Height)
        $width = $global:image.Width * $scale
        $height = $global:image.Height * $scale
        $left = $area.X + ($area.Width - $width) / 2
        $top = $area.Y + ($area.Height - $height) / 2
        $e.Graphics.InterpolationMode = 'HighQualityBicubic'
        $e.Graphics.DrawImage($global:image, [single]$left, [single]$top, [single]$width, [single]$height)
        $global:image.Dispose()
        $global:page = $global:page + 1
        $e.HasMorePages = $global:page -le $global:last
    })
    $doc.Print()
    [Console]::Out.WriteLine('done')
} catch {
    [Console]::Out.WriteLine('error ' + $_.Exception.Message)
}

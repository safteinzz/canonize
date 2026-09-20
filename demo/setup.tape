# First run: a canon that just moved, found and set up in six answers, then
# every agent's setup fixed with one F.
#
#   cd demo && vhs setup.tape

Output ../readme-assets/setup.gif

Set Shell bash
Set FontFamily "JetBrainsMono NF"
Set Theme "Catppuccin Mocha"
Set FontSize 15
Set Width 1240
Set Height 640
Set Padding 20
Set TypingSpeed 35ms

Hide
Type "./stage.sh up >/dev/null 2>&1"
Enter
Sleep 1.5s
# `exec`, so a stage that refuses to open takes the terminal with it: without it
# the outer shell survives and every keystroke after this lands in the real home.
Type "exec ./stage.sh shell"
Enter
Sleep 1.5s
Type "clear"
Enter
Sleep 300ms
Show
Type "canon"
Enter
Sleep 2s
Enter
Sleep 1.2s
Enter
Sleep 900ms
Enter
Sleep 900ms
Enter
Sleep 900ms
Enter
Sleep 900ms
Enter
Sleep 2s
Type "y"
Sleep 1.5s
Type "F"
Sleep 2s
Type "y"
Sleep 2s

Hide
Type "q"
Sleep 200ms
Type "exit"
Enter

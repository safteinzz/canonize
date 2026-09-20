# The skills tab: a skill Claude kept itself adopted with Enter, every skill
# linked to Claude with a…, then every missing link made with F.
#
#   cd demo && vhs skills.tape

Output ../readme-assets/skills.gif

Set Shell bash
Set FontFamily "JetBrainsMono NF"
Set Theme "Catppuccin Mocha"
Set FontSize 15
Set Width 1240
Set Height 640
Set Padding 20
Set TypingSpeed 35ms

Hide
Type "./stage.sh ready >/dev/null 2>&1"
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
Type "canon"
Enter
Sleep 1s
Tab
Sleep 300ms
Show
Sleep 1.5s
Type "jj"
Sleep 700ms
Enter
Sleep 1.8s
Type "y"
Sleep 1.2s
Type "a"
Sleep 1.2s
Type "j"
Sleep 600ms
Enter
Sleep 1.5s
Type "y"
Sleep 1.2s
Type "F"
Sleep 1.8s
Type "y"
Sleep 1.5s

Hide
Type "q"
Sleep 200ms
Type "exit"
Enter

# The projects tab: a broken import repointed with Enter, one house file
# added to every project with a…, and the remove choices with d.
#
#   cd demo && vhs projects.tape

Output ../readme-assets/projects.gif

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
Tab
Sleep 300ms
Show
Sleep 1.5s
Enter
Sleep 1.5s
Type "y"
Sleep 1s
Type "F"
Sleep 1.5s
Type "y"
Sleep 1.2s
Type "l"
Sleep 500ms
Type "a"
Sleep 1.2s
Type "jj"
Sleep 600ms
Enter
Sleep 1.5s
Type "y"
Sleep 1.2s
Type "d"
Sleep 1.5s
Escape
Sleep 800ms

Hide
Type "q"
Sleep 200ms
Type "exit"
Enter

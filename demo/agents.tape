# The agents tab: the list, a card opened with Enter, one line fixed, the
# skills line linking them all, and back to the list with Esc.
#
#   cd demo && vhs agents.tape

Output ../readme-assets/agents.gif

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
Show
Sleep 1.5s
Type "j"
Sleep 600ms
Type "k"
Sleep 600ms
Enter
Sleep 1s
Type "f"
Sleep 1.5s
Type "y"
Sleep 1.2s
Type "jj"
Sleep 800ms
Type "f"
Sleep 1.5s
Type "y"
Sleep 1.2s
Escape
Sleep 800ms
Type "F"
Sleep 1.5s
Type "y"
Sleep 2s

Hide
Type "q"
Sleep 200ms
Type "exit"
Enter

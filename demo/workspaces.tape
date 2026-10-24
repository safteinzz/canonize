# The workspaces tab: a folder pointed at a project with c, its card opened
# with Enter, a second project added with a, and a convention of its own
# switched on with Enter.
#
#   ./render.sh workspaces

Output ../readme-assets/workspaces.gif

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
Tab
Sleep 300ms
Show
Sleep 1.5s
Type "c"
Sleep 1s
Type "~/issues/shop"
Sleep 500ms
Enter
Sleep 500ms
Ctrl+O
Sleep 1s
Enter
Sleep 800ms
Enter
Sleep 1.8s
Enter
Sleep 1s
Type "a"
Sleep 800ms
Ctrl+O
Sleep 900ms
Type "j"
Sleep 500ms
Enter
Sleep 600ms
Enter
Sleep 1.5s
# Down the card to `support`, past the files, CONTEXT.md, both projects and rust.
Type "jjjjjj"
Sleep 800ms
Enter
Sleep 1.5s
Type "y"
Sleep 1.5s
Escape
Sleep 1.2s

Hide
Type "q"
Sleep 200ms
Type "exit"
Enter

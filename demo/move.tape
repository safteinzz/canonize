# The README's still of `canon move`: a canon that moves, with everything that
# named it repointed. Wider than the other stills, because the lines name both
# the old path and the new one.
#
#   cd demo && vhs move.tape

# VHS always records a video; with every frame hidden there is nothing to
# encode, so this one is a throwaway.
Output "/tmp/vhs-canon-move-throwaway.gif"

Set Shell bash
Set FontFamily "JetBrainsMono NF"
Set Theme "Catppuccin Mocha"
Set FontSize 15
Set Width 1440
Set Height 400
Set Padding 20

Hide
Type "./stage.sh ready >/dev/null 2>&1"
Enter
Sleep 2s
Type "exec ./stage.sh shell"
Enter
Sleep 2s
# Wired first, so the move has something to repoint.
Type "canon fix >/dev/null 2>&1"
Enter
Sleep 3s
Type "clear"
Enter
Sleep 500ms

Type "canon move canon ~/dotfiles/agents"
Enter
Sleep 2s
Show
Sleep 500ms
Screenshot ../readme-assets/move.png
Sleep 300ms

Hide
Type "exit"
Enter

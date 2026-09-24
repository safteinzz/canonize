# The README's still of `canon status`, from a staged home that is set up but
# not yet fixed. The tabs are GIFs, one tape each.
#
#   cd demo && vhs shots.tape

# VHS always records a video; with every frame hidden there is nothing to
# encode, so this one is a throwaway.
Output "/tmp/vhs-canon-shots-throwaway.gif"

Set Shell bash
Set FontFamily "JetBrainsMono NF"
Set Theme "Catppuccin Mocha"
Set FontSize 15
Set Width 1240
Set Height 620
Set Padding 20

Hide
Type "./stage.sh ready >/dev/null 2>&1"
Enter
Sleep 2s
Type "exec ./stage.sh shell"
Enter
Sleep 2s
Type "clear"
Enter
Sleep 500ms

Type "canon status"
Enter
Sleep 1.5s
Show
Sleep 500ms
Screenshot ../readme-assets/status.png
Sleep 300ms

Hide
Type "exit"
Enter

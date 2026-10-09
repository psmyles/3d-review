# Review comments

A review comment is a note about the model, saved **inside the FBX file
itself**. Whoever opens the file in 3D Review later sees the comments, and so
can anyone with a tool that reads them, because the file is still an ordinary
FBX that every other application opens.

## Writing a comment

Pick the **Comment** tool beside Select in the toolbar, or press `C`. Then:

- **Click the model** to pin a comment to that spot. The pin follows the model
  as it animates and bends; choose **Fixed in the scene** in the comment box to
  keep it at that point in space instead.
- **Click empty space** for a comment about the view as a whole.

A box opens beside the pin. Write the comment and press **Post** (or the
primary modifier with Enter); `Esc` or **Cancel** throws it away. The box
offers two extras, both on to begin with:

- **The frame** - if an animation is selected, the comment is about the frame
  on screen. Give it a second frame number to make it about a stretch of the
  animation.
- **Save the camera view** - clicking the comment later moves the view back to
  where it is now.

The first time, the box also asks for your name, which every comment and reply
is signed with. It is remembered for next time.

There are two more ways to start a comment: right-click a part in the Outliner
and choose **Comment on this part**, or press **Add a note about the file** in
the Comments tab.

## In the UV and Opt workspaces

In the **UV** workspace, a comment pinned to the model also shows on the UV
layout, at the spot its pin covers. The Comment tool works there too: click the
layout to pin a note to that point of it - "these islands overlap", say. Such a
note belongs to the UV set it was left on, and shows whenever that set is the
one on screen.

In the **Opt** workspace the comments are listed and can be read, answered and
resolved as anywhere else, and the pins sit on the original model (the left
half of the split). New comments are left in the 3D or UV workspace. An Opt
export writes the comments as they stand onto the parts they belong to, so the
optimized file carries them too.

## Replying, resolving and editing

Select a comment to see it in the Inspector. From there you can **Reply**,
**Resolve** it once it has been dealt with (or **Reopen** it), **Edit** any
message, **Move pin** to point it somewhere else, **Save current view** to
change where it takes you, or **Delete comment** (click twice). All of these
can be undone.

## Saving

Comments are part of the file, so they are saved into it: **Save Comments**
in the File menu, or the primary modifier with `S`, writes them back into the
FBX you opened. **Save Comments As** (with Shift as well) writes a copy of the
file, with the comments, somewhere else - and later saves go to that copy.

Nothing else in the file changes: the model, its materials and animations, and
everything 3D Review does not understand are written back exactly as they
were. The window title shows a `*` while there are comments to save, and
opening another file, starting over or quitting asks whether to save them
first. If the file was changed by another application since you opened it,
saving asks before writing over those changes.

## Reading comments

Open the **Comments** tab in the Outliner. It is in the 3D, UV and Opt
workspaces. Each comment has a number, the start of what was written, and
underneath that who wrote it, which part it is on and which frames it is about.

The buttons at the top choose which comments are listed: **Open** (the
default), **Resolved**, or **All**. The search box filters them by their text,
their author or the part they are on.

Clicking a comment goes to it. The part it is on is selected, the view moves to
where the comment was written from, and if it is about an animation, that
animation jumps to its frame. The Inspector shows the whole conversation.

## Pins

A comment that points at a spot on the model has a numbered pin in the view, in
the same number as its row. Open comments have yellow pins and resolved ones
green.

- A pin behind the model is drawn faded rather than hidden, so you always know a
  comment is there.
- A pin for a comment about certain frames is faint while the animation is
  elsewhere.
- Hovering a pin shows the start of the comment and who wrote it.
- Clicking a pin selects the comment and shows it in the Inspector, without
  moving the view. The Inspector's **Go to comment** button moves it.

**Show pins**, beside the filter buttons, turns the pins off and on.

## In other applications

Each comment is stored on the part it is about, as a custom property called
`ReviewComments`. Applications that read custom properties show it: a custom
attribute in Maya or 3ds Max, a custom property in Blender, a parameter in
Houdini. When such an application saves the file again, the comments go with
it, as long as it writes custom properties: Blender only does with **Custom
Properties** ticked in its FBX export options.

The `review-comments` tool installed beside the viewer prints the comments of
a file without opening the viewer, as a report or as JSON for scripts. On
Windows it is `review-comments.exe` in the install folder; on macOS it is inside
the app, at `3D Review.app/Contents/MacOS/review-comments`.

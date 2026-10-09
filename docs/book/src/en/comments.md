# Review comments

A review comment is a note about the model, saved **inside the FBX file
itself**. Whoever opens the file in 3D Review later sees the comments, and so
can anyone with a tool that reads them, because the file is still an ordinary
FBX that every other application opens.

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
a file without opening the viewer, as a report or as JSON for scripts.

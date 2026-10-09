### Review comments: the Outliner's Comments tab, the Inspector's thread view and
### the viewport pins.

## Comments tab

ui-comments-filter-open = Open
ui-comments-filter-resolved = Resolved
ui-comments-filter-all = All
ui-comments-show-pins = Show pins
    .description = Draw a numbered pin in the view for every listed comment that points at a
        spot on the model.
ui-comments-empty = No comments in this file.
ui-comments-none-open = No open comments.
ui-comments-none-resolved = No resolved comments.

# A thread's number, shown on its row and its pin.
ui-comments-number = #{ $number }
ui-comments-frame = { $clip } frame { $frame }
ui-comments-frames = { $clip } frames { $start }-{ $end }
ui-comments-row-tooltip = On { $object }
    .description = Click to go to this comment: the view moves to where it was written from,
        and the animation jumps to its frame.
ui-comments-read-only = Some comments were written by a newer version of 3D Review. They are
    shown, but can't be changed here.

## The Inspector's thread view

ui-comments-heading = Comment { $number }
ui-comments-status = Status
    .description = An open comment still needs attention. A resolved one has been dealt with,
        and is hidden from the list and the view unless you ask for it.
ui-comments-status-open = Open
ui-comments-status-resolved = Resolved
ui-comments-on = On
    .description = The object this comment is stored on in the file.
ui-comments-whole-file = The whole file
ui-comments-points-at = Points at
    .description = Where in the model this comment points.
ui-comments-anchor-surface = A spot on the surface
ui-comments-anchor-world = A point in the scene
ui-comments-anchor-uv = A point on the UV layout
ui-comments-anchor-none = Nothing in particular
ui-comments-anchor-unknown = A kind of point this version can't show
ui-comments-frames-label = Frames
    .description = The animation and frames this comment is about.
# A message's time, which is stored in UTC.
ui-comments-time-utc = { $time } UTC
ui-comments-go-to = Go to comment
    .description = Move the view to where this comment was written from, and jump the
        animation to its frame.

## Writing a comment

ui-comments-composer-on = On { $object }
ui-comments-composer-about-view = About this view
ui-comments-composer-about-file = About the whole file
ui-comments-composer-about-uv = About the UV layout
ui-comments-composer-hint = Write a comment...
ui-comments-composer-name = Your name
    .description = Comments are signed with this name. It is remembered for next time.
ui-comments-composer-name-hint = Sign your comments as...
ui-comments-composer-to-frame = to
ui-comments-composer-save-view = Save the camera view
    .description = Clicking this comment later moves the view back to where it is now.
ui-comments-composer-follow-surface = Follows the surface
    .description = The pin stays on this spot of the model as it moves and bends.
ui-comments-composer-fixed-point = Fixed in the scene
    .description = The pin stays at this point in space, whatever the model does.
ui-comments-composer-post = Post
    .description = { $modifier }+Enter
ui-comments-composer-cancel = Cancel
ui-comments-not-writable-no-file = Open a model to comment on it.
ui-comments-not-writable-unreadable = Comments can't be added to this file, because it couldn't be read for them. Only FBX 2011 and newer files can carry comments.

## Changing a comment in the Inspector

ui-comments-resolve = Resolve
    .description = Mark this comment as dealt with. Resolved comments are hidden from the list
        and the view unless you ask for them.
ui-comments-reopen = Reopen
    .description = Mark this comment as needing attention again.
ui-comments-repin = Move pin
    .description = Click the model to move this comment's pin there.
ui-comments-repin-waiting = Click the model where the pin should go.
ui-comments-update-view = Save current view
    .description = Make the current view the one this comment moves to.
ui-comments-edit = Edit
ui-comments-save-edit = Save
ui-comments-reply-hint = Write a reply...
ui-comments-reply = Reply
ui-comments-delete = Delete comment
ui-comments-delete-confirm = Click again to delete
ui-comments-thread-read-only = Written by a newer version of 3D Review, so it can't be changed here.
ui-comments-new-file-note = Add a note about the file
ui-comments-comment-on-part = Comment on this part

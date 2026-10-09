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

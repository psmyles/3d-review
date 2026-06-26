Tracy profiling

The normal way GPU profiling works
To time a GPU pass, you put two timestamp "stopwatch" markers around it — one at the start, one at the end. The GPU records its own clock at each marker. But there's a catch: the GPU runs asynchronously. When your code finishes recording the commands, the GPU hasn't actually run them yet. The timestamps don't exist until after the work is submitted to the GPU and the GPU gets around to executing it.

So the usual recipe is:

Record the pass + its timestamps.
Submit the command buffer to the GPU.
Wait a bit (or come back later), then read the timestamp values back from a GPU buffer.
The cleanest version is "submit, then immediately wait for it to finish, then read." That's the simple in-line resolve — everything for one frame happens in one tidy sequence.

Why we can't do that here
In this app, we don't own the submit. The scene is drawn inside an egui paint callback. egui hands us a command encoder, we record our passes onto it, and then egui is the one that calls queue.submit(...) and presents the frame — after our callback has already returned. We never get to say "submit now and wait."

That breaks the simple recipe in two ways:

We can't "submit then wait" because submission isn't ours to trigger.
We can't block waiting for results anyway — blocking the render thread on the GPU every frame would tank the framerate, which defeats the point of a profiler.
What we do instead — the cross-frame readback ring
Because we can only record timestamps (not control when they're submitted or wait for them), we accept that the answer arrives late and design around it:

Frame N: record the timestamps onto egui's encoder. egui submits it for us later. We do not wait.
Frame N+1: GPU is busy finishing; results may not be ready. We poll without blocking — "are you done? no? fine, move on."
Frame N+2: by now the GPU has definitely finished frame N's work, so we read frame N's timestamps and hand them to Tracy.
To pull this off we keep three sets of buffers (a "ring" of 3) — one being written this frame, one in flight, one being read — so frames don't stomp on each other's results. That's the triple-buffered readback ring. Tracy doesn't care that the numbers show up two frames late; it just lines them up under the right frame using the GPU's own clock.

The one-sentence summary
Because egui — not us — owns submit and present, we can never do the easy "submit-and-wait-right-here" GPU timing; we're forced to record timestamps now and collect them a couple of frames later through a rotating set of buffers, which is the readback ring.
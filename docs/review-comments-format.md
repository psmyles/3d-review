# Review comments in an FBX file

3D Review stores review comments **inside the FBX**. Any other application
still opens the file normally, and a script in any DCC can read the comments
with nothing but its standard library. This page is the contract for tools that
read or write them.

## Where they are

Each comment is stored on the FBX `Model` object it is about, in one
user-defined string property:

| | |
|---|---|
| Property name | `ReviewComments` |
| Type | `KString` |
| Flags | `U` (user-defined) |
| Value | JSON, described below |

In ASCII FBX terms the property line is:

```
P: "ReviewComments", "KString", "", "U", "{&quot;v&quot;:1,&quot;threads&quot;:[...]}"
```

Notes about the whole file (no particular object) are stored on the
**root-most object**: the first `Model`, in file order, whose parent is the
scene root. They are marked `"scope": "file"`.

Because it is an ordinary user property, applications that import user
properties show it: Maya and 3ds Max as an extra attribute, Blender as a custom
property on the object, Houdini as a parameter (`fbx_ReviewComments`) and a
geometry attribute, Unity to `AssetPostprocessor.OnPostprocessGameObjectWithUserProperties`.
Re-exporting carries it through where the exporter writes user properties —
Houdini does, Blender does only with **Custom Properties** ticked in its FBX
export options (off by default).

## The JSON payload

```json
{
  "v": 1,
  "threads": [
    {
      "id": "9f2c41d07ab35e18",
      "status": "open",
      "scope": "object",
      "anchor": { "kind": "surface", "face": 812, "tri": 0,
                  "bary": [0.21, 0.33, 0.46], "local": [1.2, 30.5, -4.0],
                  "topo": { "polys": 2048, "verts": 2050 } },
      "frames": { "clip": "Run", "start": 10, "end": 24 },
      "view": { "target": [0, 1.2, 0], "yaw": 0.6, "pitch": -0.25,
                "distance": 3.5, "fov": 0.785, "ortho": false },
      "messages": [
        { "author": "Ana", "time": "2026-10-08T12:00:00Z",
          "text": "The seam on the shoulder is visible at this angle." },
        { "author": "Ben", "time": "2026-10-09T09:30:00Z",
          "text": "Moved it under the strap." }
      ]
    }
  ]
}
```

### Thread

| Field | | Meaning |
|---|---|---|
| `id` | optional | Unique within the file, 16 hex digits. A reader assigns one to a thread without it. |
| `status` | optional | `open` (default) or `resolved`. |
| `scope` | optional | `object` (default) — about the object it is stored on — or `file`. |
| `anchor` | optional | Where it points (below). |
| `frames` | optional | `clip` (the animation's name), `start` and `end` frame, inclusive; `start == end` for one frame. Frames count from the clip's first frame. |
| `view` | optional | The 3D Review camera the comment was written from: the point it orbits, `yaw` and `pitch` in radians, `distance` in meters, vertical `fov` in radians. |
| `messages` | | The first opens the thread; the rest are replies, oldest first. Each has `author`, `time` (RFC 3339, UTC) and `text`. |

### Anchors

| `kind` | Fields | Meaning |
|---|---|---|
| `surface` | `face`, `tri`, `bary`, `local`, `topo` | A point on the object's mesh. `face` is the polygon index in the mesh as the file orders it, `tri` the triangle of that polygon's fan (corner 0, k+1, k+2), `bary` the barycentric weights on that triangle's corners. `local` is the same point in the object's own local space and units. `topo` records the mesh's polygon and control-point counts when the pin was placed: if they no longer match, the mesh was edited, and `local` is the one to trust. |
| `world` | `pos` | A fixed point in the scene. |
| `uv` | `set`, `uv` | A point on the UV layout named `set`. |

World positions (`pos`, a view's `target`) are in **meters**, on the file's own
axes, whatever unit the file declares.

## Rules for a tool that writes comments

- **Keep what you don't understand.** Readers ignore unknown fields, so a newer
  writer can add some; a tool that rewrites a payload should carry every field,
  thread and anchor it did not change, exactly as it found them.
- Write `"v": 1`. A reader treats a payload with a higher `v` as read-only.
- Text is plain UTF-8. In an **ASCII** FBX, a `"` inside the value is written
  `&quot;` (the FBX SDK's own escape) and an `&` as the JSON escape `\u0026`,
  since FBX has no escape for `&` itself.

## Reading them without 3D Review

The `review-comments` tool ships beside the viewer:

```
review-comments model.fbx                  # Markdown report, grouped by object
review-comments model.fbx --json           # every thread, with its object
review-comments model.fbx --status open    # only the open ones
```

It exits 0 when it listed at least one thread, 1 when there were none, and 2
when the file could not be read.

From the FBX SDK's Python bindings:

```python
import fbx, json

manager = fbx.FbxManager.Create()
importer = fbx.FbxImporter.Create(manager, "")
importer.Initialize("model.fbx", -1, manager.GetIOSettings())
scene = fbx.FbxScene.Create(manager, "")
importer.Import(scene)

def walk(node):
    prop = node.FindProperty("ReviewComments")
    if prop.IsValid():
        payload = json.loads(str(fbx.FbxPropertyString(prop).Get()))
        for thread in payload["threads"]:
            print(node.GetName(), thread["status"], thread["messages"][0]["text"])
    for i in range(node.GetChildCount()):
        walk(node.GetChild(i))

walk(scene.GetRootNode())
```

In Blender, after importing the FBX:

```python
import bpy, json
for obj in bpy.data.objects:
    if "ReviewComments" in obj:
        for thread in json.loads(obj["ReviewComments"])["threads"]:
            print(obj.name, thread["status"], thread["messages"][0]["text"])
```

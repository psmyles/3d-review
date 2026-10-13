### How an attached animation file fits the model: what does not line up
### between the two files. Each message is one line of the warning card shown
### when the file is added.

## The six directions a file's axes can point

ui-binding-axis-positive-x = +X
ui-binding-axis-negative-x = -X
ui-binding-axis-positive-y = +Y
ui-binding-axis-negative-y = -Y
ui-binding-axis-positive-z = +Z
ui-binding-axis-negative-z = -Z

# A file's axis system, as the next two messages name it: "+Z up, facing -Y".
ui-binding-axis-system = { $up } up, facing { $front }

## Mismatches

# Units are named by their symbol: "m", "cm".
ui-binding-units =
    Units differ: the animation file is in { $file } and the model in { $model }. The
    animation was converted, so it plays at the model's scale.
ui-binding-axes =
    Axes differ: the animation file is { $file } and the model { $model }. The
    animation was turned to match the model.
ui-binding-axes-unconverted =
    Axes differ: the animation file is { $file } and the model { $model }. One is the
    mirror image of the other, which turning cannot fix, so the animation plays in its
    own axes.
# { $difference } is a length in the model's unit; { $bone } is the bone that differs
# most.
ui-binding-bone-offsets = { $count ->
        [one]
            Bone '{ $bone }' is { $difference } away from where the model has it. The
            animation plays it as keyed, so the model's proportions change while it plays.
       *[other]
            { $count } bones are away from where the model has them, by up to
            { $difference } ('{ $bone }'). The animation plays them as keyed, so the
            model's proportions change while it plays.
    }
# { $mesh } is one of the meshes that differ; { $file } and { $model } are its vertex
# counts in each.
ui-binding-vertex-counts = { $count ->
        [one]
            Mesh '{ $mesh }' has { $file } vertices in the animation file and { $model } in
            the model. The animation was probably exported from another version of it.
       *[other]
            { $count } meshes have different vertex counts in the animation file, such
            as '{ $mesh }' with { $file } against the model's { $model }. The animation was
            probably exported from other versions of them.
    }
ui-binding-unmatched-bones = { $count ->
        [one]
            Bone '{ $bone }' has nothing to drive in the model, so its keys are not
            played. It was probably renamed or removed.
       *[other]
            { $count } bones, such as '{ $bone }', have nothing to drive in the model, so
            their keys are not played. They were probably renamed or removed.
    }

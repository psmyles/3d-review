# The Issues list

In the Aud workspace the Outliner has an **Issues** tab, open by default,
next to the usual **Scene** tab.

## The top of the list

- **Profile** shows the profile the model was checked against. Click it to see
  and change the profile in the Inspector; see [Profiles](profiles.md).
- **Copy summary** and **Save report** are described in
  [Reports](report.md).
- **By check** and **By object** change how the list is grouped.
- **Show passed** also lists the checks that found nothing, and the ones that
  did not run, so you can see everything that was checked. A check that did
  not run says why when you hover over it.

## By check

Each group is a kind of check (Geometry, Transforms, UVs, Skin, Density,
Naming, Hierarchy). Under it, each check that found something is listed with
how many problems it found. Click the small arrow beside a check to list the
parts that fail it.

## By object

Each part of the model is listed with the checks it fails. Findings about the
file as a whole, such as its unit, are grouped under **Whole file** at the top.

## Severity

Every finding has a severity, set by the profile:

- **Error**, a filled circle: this will cause a visible or technical problem.
- **Warning**, a triangle: this probably needs fixing.
- **Info**, a ring: worth knowing, but often deliberate.

## Picking a finding

Click a check to focus it. The rest of the model turns plain grey and the
problem is drawn in its severity's colour: tinted faces for faces, lines for
edges, and dots for points too small to see. A dot is bright where you can see
it and faint where something is in front of it, so nothing hidden is missed.

If none of the problem is on screen, the camera moves to it. Press `F` to frame
it at any time. Click a part under the check, or in the Inspector's **Parts**
list, to look at only that part's problems. Click the finding again, or press
`Esc`, to go back to the plain model.

In Select mode, clicking a highlighted problem in the viewport picks it in the
list; clicking anywhere else selects the part as usual.

In the Scene tab, each part with a finding carries a dot in the colour of its
worst one.

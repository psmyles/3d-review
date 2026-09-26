//! The toolbar's three groups must fit the bar they are laid out in.
//!
//! Each group is a fixed width, and a cluster is the sum of its groups plus the
//! spacings between them. Adding a tile to one, or splitting one group into two
//! as the Help button was, means widening the cluster that holds it - and
//! forgetting to leaves the outermost group drawn off the end of the bar with
//! nothing to say it had happened.
//!
//! The layout is pure arithmetic over the `size` tokens, so this checks the
//! arithmetic rather than the pixels: it is the tokens that have to agree.

use review_ui::theme::size;

/// The right-hand cluster in the 3D and Opt workspaces: Help, the view group,
/// the projection toggle, the side-panels toggle and the viewport-tool toggle -
/// five groups, each of which used to be three until Help was given a group of
/// its own and the Select tool was added.
#[test]
fn the_right_cluster_fits_its_reserved_width() {
    let spacing = size::TOOLBAR_GROUP_SPACING;

    // The view group gains a tile for the skeleton toggle on a rigged model, so
    // the wider of the two is what has to fit.
    for view_group in [
        size::TOOLBAR_QUAD_ICON_GROUP_WIDTH,
        size::TOOLBAR_QUINT_ICON_GROUP_WIDTH,
    ] {
        let needed = size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH
            + spacing
            + view_group
            + spacing
            + size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH
            + spacing
            + size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH
            + spacing
            + size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH;
        assert!(
            needed <= size::TOOLBAR_RIGHT_WIDTH,
            "the right toolbar cluster needs {needed}pt but TOOLBAR_RIGHT_WIDTH \
             reserves only {}pt — the outermost group would be drawn off the bar",
            size::TOOLBAR_RIGHT_WIDTH,
        );
    }
}

/// The left cluster: the menu, show-wireframe, shading, active material, and the
/// geometry-debug group.
#[test]
fn the_left_cluster_fits_its_reserved_width() {
    let spacing = size::TOOLBAR_GROUP_SPACING;

    // The material group gains a tile for the skin-weight view on a skinned mesh.
    for material_group in [
        size::TOOLBAR_MATERIAL_GROUP_WIDTH,
        size::TOOLBAR_QUINT_ICON_GROUP_WIDTH,
    ] {
        let needed = size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH
            + spacing
            + size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH
            + spacing
            + size::TOOLBAR_SHADING_GROUP_WIDTH
            + spacing
            + material_group
            + spacing
            + size::TOOLBAR_TRIPLE_ICON_GROUP_WIDTH;
        assert!(
            needed <= size::TOOLBAR_LEFT_WIDTH,
            "the left toolbar cluster needs {needed}pt but TOOLBAR_LEFT_WIDTH \
             reserves only {}pt",
            size::TOOLBAR_LEFT_WIDTH,
        );
    }
}

/// A group's declared width has to hold the tiles it is named for: four points of
/// padding, then a tile each with two points between them.
#[test]
fn every_icon_group_width_holds_its_tiles() {
    const PADDING: f32 = 4.0;
    const GAP: f32 = 2.0;

    let cases = [
        (1, size::TOOLBAR_SINGLE_ICON_GROUP_WIDTH),
        (3, size::TOOLBAR_TRIPLE_ICON_GROUP_WIDTH),
        (4, size::TOOLBAR_QUAD_ICON_GROUP_WIDTH),
        (4, size::TOOLBAR_SHADING_GROUP_WIDTH),
        (5, size::TOOLBAR_QUINT_ICON_GROUP_WIDTH),
    ];

    for (tiles, width) in cases {
        let needed = PADDING + tiles as f32 * size::TOOLBAR_ICON_SIZE + (tiles - 1) as f32 * GAP;
        assert!(
            (width - needed).abs() < 0.5,
            "a {tiles}-icon group needs {needed}pt but its token is {width}pt",
        );
    }
}

/// The three clusters plus the centred mode group must not overlap at the
/// narrowest window the chrome is expected to work in.
#[test]
fn the_clusters_do_not_overlap_at_a_small_window() {
    // A 1280pt-wide window: the smallest a 3D viewer is realistically used at.
    const BAR_WIDTH: f32 = 1280.0;
    let usable = BAR_WIDTH - size::OVERLAY_MARGIN * 2.0;

    let left_end = size::TOOLBAR_LEFT_WIDTH;
    let right_start = usable - size::TOOLBAR_RIGHT_WIDTH;
    let centre_start = usable / 2.0 - size::TOOLBAR_MODE_GROUP_WIDTH / 2.0;
    let centre_end = usable / 2.0 + size::TOOLBAR_MODE_GROUP_WIDTH / 2.0;

    assert!(
        left_end < centre_start,
        "the left cluster ({left_end}pt) reaches the centred mode group \
         ({centre_start}pt) at a {BAR_WIDTH}pt window",
    );
    assert!(
        centre_end < right_start,
        "the centred mode group ends at {centre_end}pt, past where the right \
         cluster starts ({right_start}pt), at a {BAR_WIDTH}pt window",
    );
}

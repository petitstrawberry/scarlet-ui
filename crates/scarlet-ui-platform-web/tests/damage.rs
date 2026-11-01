// Exercise the exact production helper on native hosts without importing the
// wasm-only platform or requiring a browser, adapter, or DOM dependencies.
#[path = "../src/damage.rs"]
mod damage;

use damage::render_areas;

#[test]
fn absent_damage_requests_the_full_target() {
    assert_eq!(render_areas(640, 480, None), [(0, 0, 640, 480)]);
    // Preserve the selector's existing zero-extent semantics; the renderer
    // rejects unusable target sizes rather than this helper inventing pixels.
    assert_eq!(render_areas(0, 0, None), [(0, 0, 0, 0)]);
}

#[test]
fn partial_damage_never_expands_to_clear_unchanged_widgets() {
    let dirty = [(10, 20, 30, 40), (300, 200, 15, 10)];
    assert_eq!(render_areas(640, 480, Some(&dirty)), dirty);
    assert!(render_areas(640, 480, Some(&[])).is_empty());
}

#[test]
fn damage_clamps_to_target_and_discards_empty_or_outside_rectangles() {
    let dirty = [
        (90, 70, 30, 40),
        (100, 0, 1, 1),
        (0, 80, 1, 1),
        (150, 150, 2, 2),
        (0, 0, 0, 5),
        (0, 0, 5, 0),
    ];
    assert_eq!(render_areas(100, 80, Some(&dirty)), [(90, 70, 10, 10)]);
    assert!(render_areas(0, 80, Some(&dirty)).is_empty());
    assert!(render_areas(100, 0, Some(&dirty)).is_empty());
}

#[test]
fn overflowing_damage_extents_saturate_instead_of_wrapping() {
    let dirty = [(10, 20, u32::MAX, u32::MAX)];
    assert_eq!(render_areas(100, 80, Some(&dirty)), [(10, 20, 90, 60)]);
    assert_eq!(
        render_areas(
            u32::MAX,
            u32::MAX,
            Some(&[(u32::MAX - 1, u32::MAX - 2, 9, 9)])
        ),
        [(u32::MAX - 1, u32::MAX - 2, 1, 2)]
    );
    assert!(render_areas(100, 80, Some(&[(u32::MAX, u32::MAX, 9, 9)])).is_empty());
}

#[test]
fn separate_or_overlapping_regions_are_not_replaced_with_a_bounding_clear() {
    let dirty = [(1, 1, 2, 2), (7, 7, 2, 2), (2, 2, 2, 2)];
    assert_eq!(render_areas(10, 10, Some(&dirty)), dirty);
}

#[test]
fn retained_target_pixels_outside_damage_are_preserved() {
    // Model a retained target containing an unchanged widget (7). The partial
    // paint list can repaint only the dirty widget (9), not reconstruct 7s.
    let mut pixels = [7; 8 * 6];
    for (x, y, width, height) in render_areas(8, 6, Some(&[(2, 1, 3, 2)])) {
        for row in y..y + height {
            for col in x..x + width {
                pixels[(row * 8 + col) as usize] = 9;
            }
        }
    }
    for row in 0..6 {
        for col in 0..8 {
            let expected = if (1..3).contains(&row) && (2..5).contains(&col) {
                9
            } else {
                7
            };
            assert_eq!(pixels[row * 8 + col], expected);
        }
    }
}

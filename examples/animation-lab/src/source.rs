//! Readable excerpts from the code that actually runs each demo.
//!
//! These are excerpts, not standalone programs. The viewer can open each
//! section's complete source file for imports, helpers, and renderer plumbing.
//! Missing or ambiguous anchors deliberately panic instead of displaying stale
//! examples. Keep this module std-only so its catalog can be tested with rustc.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExampleId {
    Donut,
    Zigzag,
    Jelly,
    Confetti,
    Explicit,
    Implicit,
    Layout,
    Charts,
    Theme,
}

#[derive(Debug)]
pub(crate) struct Example {
    pub title: &'static str,
    pub summary: &'static str,
    pub steps: &'static [&'static str],
    pub sections: Vec<CodeSection>,
}

#[derive(Debug)]
pub(crate) struct CodeSection {
    pub title: &'static str,
    pub file: SourceFile,
    pub code: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceFile {
    Play,
    Paint,
    Basics,
    Layout,
}

impl SourceFile {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Play => "play.rs",
            Self::Paint => "paint.rs",
            Self::Basics => "basics.rs",
            Self::Layout => "layout.rs",
        }
    }

    pub(crate) const fn contents(self) -> &'static str {
        match self {
            Self::Play => include_str!("play.rs"),
            Self::Paint => include_str!("paint.rs"),
            Self::Basics => include_str!("basics.rs"),
            Self::Layout => include_str!("layout.rs"),
        }
    }
}

impl ExampleId {
    pub(crate) fn example(self) -> Example {
        match self {
            Self::Donut => custom_example(
                "Donut Club",
                "A looping phase moves the satellites while a separate value fills the ring.",
                &[
                    "Load again sets the State<Model> progress target to 1.0 inside a 2200 ms ease-in-out transaction. A new load resets the displayed progress first.",
                    "StageRenderer owns AnimatedValue tracks. Its linear phase repeats every 2600 ms; Speed up halves that period. Phase 0..1 becomes an angle in radians.",
                    "The custom ElementRenderObject advances the tracks and paints their displayed values. Open play.rs for the RenderElement hookup and pause/reset handling.",
                    "The inner arc sweeps through TAU * progress. Sine and cosine place the moving dots around the same center.",
                ],
                "Draw the ring and orbiting dots",
                "fn orbit(&mut self, angle: f32, progress: f32)",
                false,
            ),
            Self::Zigzag => custom_example(
                "Zigzag Express",
                "Use one progress value to reveal a path and move a star along it.",
                &[
                    "Load again changes the progress target to 1.0. The custom renderer retargets its displayed value using the 2200 ms ease-in-out transaction.",
                    "A separate linear phase repeats every 3100 ms. It keeps the beads and star moving after the progress reaches the end.",
                    "AnimatedValue tracks belong to StageRenderer, whose advance_visual_animation and paint methods connect them to the framework clock. The full play.rs shows the hookup.",
                    "zigzag_point maps 0..1 onto six slopes. The drawing reveals 36 short segments up to progress, then puts the star at zigzag_point(progress).",
                ],
                "Map progress onto the zigzag",
                "fn zigzag(&mut self, angle: f32, progress: f32)",
                false,
            ),
            Self::Jelly => custom_example(
                "Jelly Buddy",
                "Turn a looping number into a hop, a squash, a shadow, and a wobbling outline.",
                &[
                    "The energy bar follows the same State<Model> progress target: Load again requests a 2200 ms ease-in-out fill.",
                    "Jelly uses a 1450 ms linear repeating phase. Speed up halves that period; the renderer's running flag pauses advancement.",
                    "This is custom drawing: StageRenderer advances AnimatedValue tracks and passes their displayed values to paint. Open play.rs for the RenderElement and update implementation.",
                    "hop = (1 - cos(angle)) / 2 gives a smooth rise and landing. That value changes the ellipse radii, body height, and shadow; a sampled path adds the wobble.",
                ],
                "Shape the hop and soft body",
                "fn jelly(&mut self, angle: f32, progress: f32)",
                false,
            ),
            Self::Confetti => custom_example(
                "Hooray Machine",
                "A replayable burst sends stars, hearts, and confetti out from the center.",
                &[
                    "Celebrate increments burst_nonce in State<Model>. StageRenderer detects the new request, resets burst to 0.0, and eases it to 1.0 over 1600 ms.",
                    "A separate 3400 ms repeating phase keeps the background stars alive. The one-shot burst and the repeating phase can run together.",
                    "The custom renderer advances and paints AnimatedValue tracks on the framework clock. Finishing a load also calls start_burst; open play.rs for the full renderer hookup.",
                    "The drawing expands 28 particles using spread, fades them using alpha, and chooses a star, heart, or pill with i % 3. The burst is finished at 1.0.",
                ],
                "Expand and fade the particles",
                "fn party(&mut self, angle: f32, burst: f32)",
                true,
            ),
            Self::Explicit => basics_example(false),
            Self::Implicit => basics_example(true),
            Self::Layout => layout_example(),
            Self::Charts => charts_example(),
            Self::Theme => theme_example(),
        }
    }
}

fn custom_example(
    title: &'static str,
    summary: &'static str,
    steps: &'static [&'static str],
    drawing_title: &'static str,
    drawing_anchor: &str,
    burst: bool,
) -> Example {
    let play = SourceFile::Play;
    let paint = SourceFile::Paint;
    let update = item(play, "fn update(&mut self, view: &PlaygroundStage)");
    let request = if burst {
        section(
            "State request and the 1600 ms burst",
            play,
            &[
                item(play, "fn celebrate(&self)"),
                statement(update, "let new_burst ="),
                block(update, "if new_burst {"),
                item(play, "fn start_burst(&mut self)"),
            ],
        )
    } else {
        section(
            "State request and the 2200 ms progress track",
            play,
            &[
                item(play, "fn load(&self)"),
                statement(update, "let transaction ="),
                block(update, "if new_load {"),
                statement(update, "changed |= self"),
            ],
        )
    };
    let drawing = item(paint, drawing_anchor);
    let drawing = if title == "Zigzag Express" {
        section(
            drawing_title,
            paint,
            &[item(paint, "fn zigzag_point(t: f32)"), drawing],
        )
    } else {
        section(drawing_title, paint, &[drawing])
    };
    Example {
        title,
        summary,
        steps,
        sections: vec![
            request,
            section(
                "Renderer tracks and a repeating phase",
                play,
                &[
                    item(play, "struct StageRenderer {"),
                    item(play, "fn start_clock(&mut self)"),
                ],
            ),
            section(
                "Advance the tracks, then paint their values",
                play,
                &[
                    item(
                        play,
                        "fn advance_visual_animation(&mut self, delta: Duration)",
                    ),
                    item(play, "fn paint(&self, context:"),
                ],
            ),
            drawing,
        ],
    }
}

fn basics_example(implicit: bool) -> Example {
    let file = SourceFile::Basics;
    let content = method(file, "impl Basics {", "fn content(&self)");
    let stage = item(file, "fn motion_stage(");
    let tile = through_statement(stage, "let small =", "let tile =");
    if implicit {
        Example {
            title: "Implicit view scope",
            summary: "Attach an animation to a view and tell it which value to watch.",
            steps: &[
                "Each panel owns a State<bool>. The button flips implicit with an ordinary state update.",
                "The tile describes both endpoints with frame, background, offset, and opacity. State changes immediately; the displayed properties interpolate.",
                "The .animation(..., implicit) scope watches that bool and applies 1200 ms ease-in-out when it changes. Reset uses without_animation to settle immediately.",
            ],
            sections: vec![
                section(
                    "Keep the target in state",
                    file,
                    &[
                        item(file, "pub(crate) struct Basics {"),
                        item(file, "impl Default for Basics {"),
                    ],
                ),
                section("Describe the tile's endpoints", file, &[tile]),
                section(
                    "Watch the value and wire the controls",
                    file,
                    &[statement(content, "let second = panel(")],
                ),
            ],
        }
    } else {
        Example {
            title: "Explicit mutation",
            summary: "Wrap a state change in with_animation to animate its visual result.",
            steps: &[
                "The explicit State<bool> selects the target. with_animation applies a 1400 ms spring with damping 0.65 to the enclosed update.",
                "frame, background, offset, and opacity read the target bool. Their displayed values move toward the new endpoints when the view rebuilds.",
                "Animate / reverse calls toggle_explicit. Tap again during motion to retarget it; Reset uses without_animation to settle at A immediately.",
            ],
            sections: vec![
                section(
                    "State and the explicit spring",
                    file,
                    &[
                        item(file, "pub(crate) struct Basics {"),
                        item(file, "fn toggle_explicit(&self)"),
                    ],
                ),
                section("Describe the tile's endpoints", file, &[tile]),
                section(
                    "Wire Animate and Reset",
                    file,
                    &[statement(content, "let first = panel(")],
                ),
            ],
        }
    }
}

fn layout_example() -> Example {
    let file = SourceFile::Layout;
    let content = method(file, "impl LayoutLab {", "fn content(&self)");
    let card = method(file, "impl MetricCard {", "fn content(&self)");
    let pose = item(file, "fn card_pose(");
    Example {
        title: "Cards and layouts",
        summary: "Keep the same cards while their target sizes and positions change.",
        steps: &[
            "The model selects Board, Rows, or Spotlight. The actions change that model; model.key() tells the parent animation scope what changed.",
            "Each card keeps its id as its key. The shared scope uses an 850 ms spring with damping 0.82, so existing cards move to their new targets.",
            "card_pose calculates the geometry for each layout. The Rows branch below uses 116 px rows on wide content and 208 px stacked cards on narrower content.",
            "The card applies the pose through frame and offset. Compact cards dim their charts while keeping button labels fully opaque. Open layout.rs for the Board and Spotlight poses and the complete card body.",
        ],
        sections: vec![
            section(
                "Choose a layout in state",
                file,
                &[
                    item(file, "struct Model {"),
                    item(file, "fn layout(&self, layout: Layout)"),
                    item(file, "fn focus(&self, id: usize)"),
                ],
            ),
            section(
                "Preserve card identity and share a spring",
                file,
                &[
                    item(file, "impl Model {"),
                    statement(content, "let cards ="),
                    parent_animation(),
                ],
            ),
            section(
                "Rows: compute each card's destination",
                file,
                &[
                    statement(pose, "let slot ="),
                    block(pose, "Layout::Rows => {"),
                ],
            ),
            section(
                "Apply the pose to the card",
                file,
                &[block_tail(card, ".frame(pose.width, pose.height)")],
            ),
        ],
    }
}

fn charts_example() -> Example {
    let file = SourceFile::Layout;
    let chart = method(file, "impl MiniChart {", "fn content(&self)");
    Example {
        title: "Refreshing charts",
        summary: "A revision changes the data; each bar gets its own easing duration.",
        steps: &[
            "Refresh increments revision in State<Model>. MiniChart receives that revision and derives eight bar heights from it.",
            "The bar's frame and offset keep its base on the chart baseline. All eight bars start together; their ease-in-out durations range from 560 to 1050 ms in 70 ms steps.",
            "Each bar keeps a stable key and watches revision. A data refresh uses the bar's easing; a layout change with unchanged revision inherits the parent's 850 ms spring.",
        ],
        sections: vec![
            section(
                "Request new data",
                file,
                &[
                    item(file, "fn refresh(&self)"),
                    item(file, "struct MiniChart {"),
                ],
            ),
            section(
                "Calculate and animate the bars",
                file,
                &[through_statement(chart, "let gap =", "let bars =")],
            ),
            section(
                "Inherit layout motion from the parent",
                file,
                &[item(file, "impl Model {"), parent_animation()],
            ),
        ],
    }
}

fn theme_example() -> Example {
    let file = SourceFile::Layout;
    let card = method(file, "impl MetricCard {", "fn content(&self)");
    Example {
        title: "Changing the theme",
        summary: "Change a palette in state and animate the resulting background colors.",
        steps: &[
            "The theme action flips model.dark inside with_animation: an 850 ms spring with damping 0.82 includes the full-window surface. model.key() also triggers the matching view scope for the cards.",
            "Palette::new returns the target colors for the light or dark theme. Views rebuild with those new colors.",
            "Background modifiers interpolate the canvas and card surfaces. Text colors update immediately. Selected cards also choose a theme-specific surface.",
        ],
        sections: vec![
            section(
                "Toggle the theme and track the change",
                file,
                &[item(file, "fn theme(&self)"), item(file, "impl Model {")],
            ),
            section(
                "Choose the target palette",
                file,
                &[item(file, "struct Palette {"), item(file, "impl Palette {")],
            ),
            section(
                "Apply colors inside the animation scope",
                file,
                &[
                    parent_animation(),
                    statement(card, "let p ="),
                    statement(card, "let surface ="),
                    block_tail(card, ".frame(pose.width, pose.height)"),
                ],
            ),
        ],
    }
}

fn parent_animation() -> &'static str {
    block_tail(
        method(SourceFile::Layout, "impl LayoutLab {", "fn content(&self)"),
        ".background(p.canvas)",
    )
}

fn section(title: &'static str, file: SourceFile, excerpts: &[&str]) -> CodeSection {
    assert!(!excerpts.is_empty(), "empty Animation Lab source section");
    CodeSection {
        title,
        file,
        code: excerpts
            .iter()
            .map(|excerpt| dedent(excerpt))
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

fn item(file: SourceFile, anchor: &str) -> &'static str {
    block(file.contents(), anchor)
}

fn method(file: SourceFile, implementation: &str, anchor: &str) -> &'static str {
    block(item(file, implementation), anchor)
}

/// Locate exactly one known declaration/expression, retaining its indentation.
fn anchor_start(source: &str, anchor: &str) -> usize {
    assert!(!anchor.is_empty(), "empty Animation Lab source anchor");
    let mut matches = source.match_indices(anchor);
    let (start, _) = matches
        .next()
        .unwrap_or_else(|| panic!("missing Animation Lab source anchor: {anchor:?}"));
    assert!(
        matches.next().is_none(),
        "ambiguous Animation Lab source anchor: {anchor:?}"
    );
    let line = source[..start].rfind('\n').map_or(0, |index| index + 1);
    if source[line..start].trim().is_empty() {
        line
    } else {
        start
    }
}

/// Extract a complete declaration or branch, ignoring braces in comments/literals.
fn block<'a>(source: &'a str, anchor: &str) -> &'a str {
    let start = anchor_start(source, anchor);
    let mut depth = 0;
    let mut opened = false;
    for (index, byte) in (CodeBytes { source, at: start }) {
        match byte {
            b'{' => {
                opened = true;
                depth += 1;
            }
            b'}' if opened => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..=index];
                }
            }
            _ => {}
        }
    }
    panic!("unclosed Animation Lab source block: {anchor:?}");
}

/// Include a whole statement, even when closures contain their own semicolons.
fn statement<'a>(source: &'a str, anchor: &str) -> &'a str {
    let start = anchor_start(source, anchor);
    let mut delimiters = Vec::new();
    for (index, byte) in (CodeBytes { source, at: start }) {
        match byte {
            b'(' | b'[' | b'{' => delimiters.push(byte),
            b')' | b']' | b'}' => {
                let expected = match byte {
                    b')' => b'(',
                    b']' => b'[',
                    _ => b'{',
                };
                assert_eq!(
                    delimiters.pop(),
                    Some(expected),
                    "unbalanced Animation Lab statement: {anchor:?}"
                );
            }
            b';' if delimiters.is_empty() => return &source[start..=index],
            _ => {}
        }
    }
    panic!("unterminated Animation Lab statement: {anchor:?}");
}

fn through_statement<'a>(source: &'a str, first: &str, last: &str) -> &'a str {
    let start = anchor_start(source, first);
    let last_start = anchor_start(source, last);
    assert!(start <= last_start, "reversed Animation Lab source excerpt");
    &source[start..last_start + statement(source, last).len()]
}

/// The final modifier chain of an already extracted function.
fn block_tail<'a>(source: &'a str, anchor: &str) -> &'a str {
    let start = anchor_start(source, anchor);
    let body = source
        .strip_suffix('}')
        .expect("Animation Lab block must end with a closing brace");
    body[start..].trim_end()
}

fn dedent(source: &str) -> String {
    let indentation = source
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .expect("empty Animation Lab source excerpt");
    source
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                ""
            } else {
                &line[indentation..]
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A small lexer for excerpt boundaries, not a Rust parser. Source files are
/// compiled by the Lab itself. Skip comments, quoted/raw strings, and character
/// literals while preserving lifetimes such as `'_` and `'static` as code.
struct CodeBytes<'a> {
    source: &'a str,
    at: usize,
}

impl Iterator for CodeBytes<'_> {
    type Item = (usize, u8);

    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.source.as_bytes();
        while self.at < bytes.len() {
            let at = self.at;
            if bytes[at..].starts_with(b"//") {
                self.at += bytes[at..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .unwrap_or(bytes.len() - at);
                continue;
            }
            if bytes[at..].starts_with(b"/*") {
                self.at += 2;
                let mut depth = 1;
                while self.at < bytes.len() && depth != 0 {
                    if bytes[self.at..].starts_with(b"/*") {
                        depth += 1;
                        self.at += 2;
                    } else if bytes[self.at..].starts_with(b"*/") {
                        depth -= 1;
                        self.at += 2;
                    } else {
                        self.at += 1;
                    }
                }
                assert_eq!(depth, 0, "unclosed comment in Animation Lab source");
                continue;
            }
            if bytes[at] == b'r' {
                let mut quote = at + 1;
                while bytes.get(quote) == Some(&b'#') {
                    quote += 1;
                }
                if bytes.get(quote) == Some(&b'"') {
                    let hashes = quote - at - 1;
                    self.at = quote + 1;
                    loop {
                        assert!(self.at < bytes.len(), "unclosed raw string in Lab source");
                        if bytes[self.at] == b'"'
                            && bytes.get(self.at + 1..self.at + 1 + hashes)
                                == Some(&bytes[at + 1..quote])
                        {
                            self.at += 1 + hashes;
                            break;
                        }
                        self.at += 1;
                    }
                    continue;
                }
            }
            if bytes[at] == b'"' {
                self.at += 1;
                loop {
                    assert!(self.at < bytes.len(), "unclosed string in Lab source");
                    match bytes[self.at] {
                        b'\\' => self.at += 2,
                        b'"' => {
                            self.at += 1;
                            break;
                        }
                        _ => self.at += 1,
                    }
                }
                continue;
            }
            if bytes[at] == b'\'' {
                let end = if bytes.get(at + 1) == Some(&b'\\') {
                    match bytes.get(at + 2) {
                        Some(b'u') if bytes.get(at + 3) == Some(&b'{') => bytes[at + 4..]
                            .iter()
                            .position(|byte| *byte == b'}')
                            .map(|index| at + 5 + index),
                        Some(b'x') => Some(at + 5),
                        Some(_) => Some(at + 3),
                        None => None,
                    }
                } else {
                    self.source[at + 1..]
                        .chars()
                        .next()
                        .map(|character| at + 1 + character.len_utf8())
                };
                if let Some(end) = end {
                    if bytes.get(end) == Some(&b'\'') {
                        self.at = end + 1;
                        continue;
                    }
                }
            }
            self.at += 1;
            return Some((at, bytes[at]));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDS: [ExampleId; 9] = [
        ExampleId::Donut,
        ExampleId::Zigzag,
        ExampleId::Jelly,
        ExampleId::Confetti,
        ExampleId::Explicit,
        ExampleId::Implicit,
        ExampleId::Layout,
        ExampleId::Charts,
        ExampleId::Theme,
    ];
    const FILES: [SourceFile; 4] = [
        SourceFile::Play,
        SourceFile::Paint,
        SourceFile::Basics,
        SourceFile::Layout,
    ];

    #[test]
    fn every_example_has_short_real_source_sections() {
        for (index, id) in IDS.into_iter().enumerate() {
            assert!(!IDS[..index].contains(&id));
            let example = id.example();
            assert!(!example.title.is_empty(), "{id:?}");
            assert!(!example.summary.is_empty(), "{id:?}");
            assert!(!example.steps.is_empty(), "{id:?}");
            assert!(example.steps.iter().all(|step| !step.trim().is_empty()));
            assert!((2..=4).contains(&example.sections.len()), "{id:?}");
            for section in example.sections {
                assert!(!section.title.is_empty(), "{id:?}");
                assert!(!section.code.trim().is_empty(), "{id:?}");
                assert!(
                    section.code.lines().count() <= 64,
                    "{id:?}: {} is no longer a short excerpt",
                    section.title
                );
                // Extraction only removes indentation and joins actual excerpts.
                // No fabricated lines, explanatory comments, or ellipses enter code.
                let original: Vec<_> = section.file.contents().lines().map(str::trim).collect();
                for line in section.code.lines().filter(|line| !line.trim().is_empty()) {
                    assert!(
                        original
                            .iter()
                            .any(|source_line| source_line.contains(line.trim())),
                        "{id:?}: {:?} is absent from {}",
                        line,
                        section.file.name()
                    );
                }
            }
        }
    }

    #[test]
    fn full_file_links_cover_all_four_embedded_sources() {
        let expected = [
            ("play.rs", include_str!("play.rs")),
            ("paint.rs", include_str!("paint.rs")),
            ("basics.rs", include_str!("basics.rs")),
            ("layout.rs", include_str!("layout.rs")),
        ];
        let mut used = Vec::new();
        for id in IDS {
            used.extend(
                id.example()
                    .sections
                    .into_iter()
                    .map(|section| section.file),
            );
        }
        for (file, (name, contents)) in FILES.into_iter().zip(expected) {
            assert_eq!(file.name(), name);
            assert_eq!(file.contents(), contents);
            assert!(contents.lines().count() > 100);
            assert!(used.contains(&file), "unmapped source: {name}");
        }
    }

    #[test]
    fn each_example_keeps_its_specific_technique_and_parameters() {
        let cases: [(ExampleId, &[&str]); 9] = [
            (
                ExampleId::Donut,
                &[
                    "fn orbit(",
                    "TAU * progress",
                    "2200",
                    "Scene::Orbit => 2600",
                ],
            ),
            (
                ExampleId::Zigzag,
                &[
                    "fn zigzag_point(",
                    "zigzag_point(progress)",
                    "Scene::Zigzag => 3100",
                ],
            ),
            (
                ExampleId::Jelly,
                &[
                    "fn jelly(",
                    "let hop =",
                    "let blob = sample(64",
                    "Scene::Jelly => 1450",
                ],
            ),
            (
                ExampleId::Confetti,
                &[
                    "fn celebrate(",
                    "fn start_burst(",
                    "1600",
                    "for i in 0..28",
                    "self.heart(",
                ],
            ),
            (
                ExampleId::Explicit,
                &[
                    "with_animation(",
                    "1400",
                    "0.65",
                    "without_animation(",
                    ".opacity(",
                ],
            ),
            (
                ExampleId::Implicit,
                &[
                    ".animation(",
                    "1200",
                    "toggle_implicit.update",
                    "without_animation(",
                ],
            ),
            (
                ExampleId::Layout,
                &[
                    "Layout::Rows =>",
                    ".key(id as u64)",
                    ".offset(pose.x, pose.y)",
                    "850",
                    "0.82",
                ],
            ),
            (
                ExampleId::Charts,
                &[
                    "fn refresh(",
                    "560 + i as u64 * 70",
                    ".key(i as u64)",
                    "self.revision",
                ],
            ),
            (
                ExampleId::Theme,
                &[
                    "fn theme(",
                    "s.dark = !s.dark",
                    "impl Palette",
                    ".background(surface)",
                    ".background(p.canvas)",
                ],
            ),
        ];
        for (id, needles) in cases {
            let code = id
                .example()
                .sections
                .into_iter()
                .map(|section| section.code)
                .collect::<Vec<_>>()
                .join("\n");
            for needle in needles {
                assert!(code.contains(needle), "{id:?} lost {needle:?}");
            }
        }
    }

    #[test]
    fn block_extraction_ignores_literals_comments_and_lifetimes() {
        let source = r####"
            fn example<'a>(value: &'a str) {
                let quoted = "} \" {";
                let raw = r##"} { \""##;
                let byte_string = br#"}"#;
                let chars = ('{', '}', '\'', '\\', '\u{7b}', '\x7d', '♥');
                /* } /* nested { */ } */
                // }
                if value.is_empty() { return; }
            }
            fn next() {}
        "####;
        let excerpt = dedent(block(source, "fn example<'a>"));
        assert!(excerpt.starts_with("fn example<'a>"));
        assert!(excerpt.ends_with("\n}"));
        assert!(excerpt.contains("if value.is_empty() { return; }"));
        assert!(!excerpt.contains("fn next"));
    }

    #[test]
    fn statement_extraction_keeps_nested_closures_and_semicolons() {
        let source = r#"
            let first = 1;
            let values = [1, 2].map(|value| {
                let text = "; }";
                (value, text)
            });
            let later = 2;
        "#;
        let excerpt = dedent(through_statement(source, "let first =", "let values ="));
        assert!(excerpt.starts_with("let first = 1;"));
        assert!(excerpt.ends_with("});"));
        assert!(excerpt.contains("(value, text)"));
        assert!(!excerpt.contains("let later"));
    }

    #[test]
    #[should_panic(expected = "missing Animation Lab source anchor")]
    fn a_removed_anchor_fails_loudly() {
        block("fn renamed() {}", "fn original()");
    }

    #[test]
    #[should_panic(expected = "ambiguous Animation Lab source anchor")]
    fn a_duplicate_anchor_fails_loudly() {
        block("impl A { fn new() {} } impl B { fn new() {} }", "fn new()");
    }

    #[test]
    #[should_panic(expected = "unclosed Animation Lab source block")]
    fn an_unclosed_block_fails_loudly() {
        block("fn unfinished() { let message = \"}\";", "fn unfinished()");
    }
}

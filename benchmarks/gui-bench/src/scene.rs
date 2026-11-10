//! Native declarative components. Each logical node owns its State subscription.
use bench_core::{Scenario, StyleComplexity, Topology, Workload};
use scarlet_ui::{
    element::{ComponentElement, Element, ElementRenderObject, LayoutConstraints, RenderElement},
    geometry::{Alignment, EdgeInsets, Point, Size},
    prelude::*,
    state::Listenable,
    view::ViewKey,
    views::{ElevationRole, Surface, SurfaceRole, containers::ViewTuple},
};
use std::{any::Any, rc::Rc};

#[derive(Clone, Default)]
struct Children(Vec<Box<dyn View>>);

impl ViewTuple for Children {
    fn create_elements(&self) -> Vec<Box<dyn Element>> {
        self.0.iter().map(|view| view.create_element()).collect()
    }
    fn clone_views(&self) -> Vec<Box<dyn View>> {
        self.0.clone()
    }
    fn collect_listenables<'a>(&'a self, _: &mut Vec<&'a dyn Listenable>) {
        // Child components own their subscriptions; avoid subscribing ancestors.
    }
}

#[derive(Clone)]
struct Node {
    id: usize,
    key: ViewKey,
    spec: Rc<Scenario>,
    tick: State<u64>,
    children: Rc<Children>,
}

impl Node {
    fn body(&self) -> Box<dyn View> {
        let tick = self.tick.get();
        let changed = tick % 2 == 1;
        let label = if self.spec.workload == Workload::TextUpdate && tick > 0 {
            format!("item {:05} · {tick:04}", self.id)
        } else {
            format!("item {:05}", self.id)
        };
        let mut children = vec![Box::new(
            Text::new(label)
                .font_size(12.0)
                .color(Color::rgb(232, 238, 252)),
        ) as Box<dyn View>];
        children.extend(self.children.0.iter().cloned());
        let layout_changed = changed && self.spec.workload == Workload::LayoutUpdate;
        let paint_changed = changed
            && matches!(
                self.spec.workload,
                Workload::PaintUpdate | Workload::InputLatency
            );
        let content = VStack::new(Children(children))
            .alignment(Alignment::Leading)
            .padding_insets(EdgeInsets::new(
                if layout_changed { 12.0 } else { 4.0 },
                1.0,
                4.0,
                1.0,
            ));
        let surface = Surface::new(content, SurfaceRole::Structural)
            .fill(if paint_changed {
                Color::rgb(53, 125, 116)
            } else {
                Color::rgb(24, 36, 58)
            })
            .corner_radius(if self.spec.style == StyleComplexity::Decorated {
                4.0
            } else {
                0.0
            })
            .bordered(self.spec.style == StyleComplexity::Decorated)
            .border_color(Color::rgb(59, 77, 107))
            .elevation(if self.spec.style == StyleComplexity::Decorated {
                ElevationRole::Raised
            } else {
                ElevationRole::Flat
            });
        Box::new(surface)
    }
}

impl View for Node {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(self.clone(), Self::body))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.tick]
    }
    fn key(&self) -> Option<&ViewKey> {
        Some(&self.key)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
pub struct Document {
    spec: Rc<Scenario>,
    ticks: Rc<Vec<State<u64>>>,
    reduced: State<bool>,
}

impl Document {
    pub fn new(spec: Scenario) -> Self {
        Self {
            ticks: Rc::new(
                (0..spec.nodes)
                    .map(|_| State::new(scarlet_ui::generate_state_id(), 0))
                    .collect(),
            ),
            reduced: State::new(scarlet_ui::generate_state_id(), false),
            spec: Rc::new(spec),
        }
    }

    pub fn stimulate(&self, sequence: u64) {
        match self.spec.workload {
            Workload::StructuralUpdate => self.reduced.set(sequence % 2 == 1),
            Workload::PaintUpdate
            | Workload::LayoutUpdate
            | Workload::TextUpdate
            | Workload::InputLatency => {
                for id in bench_core::affected_indices(&self.spec, sequence) {
                    self.ticks[id].update(|tick| *tick += 1);
                }
            }
            _ => {}
        }
    }

    fn body(&self) -> Box<dyn View> {
        let visible = if self.reduced.get() {
            self.spec
                .nodes
                .saturating_sub((self.spec.nodes / 10).max(1))
        } else {
            self.spec.nodes
        };
        let mut descendants = vec![Vec::<Box<dyn View>>::new(); visible];
        let mut roots = Vec::<Box<dyn View>>::new();
        // Build bottom-up so constructing a deep topology does not recurse.
        for id in (0..visible).rev() {
            let mut children = std::mem::take(&mut descendants[id]);
            children.reverse();
            let node = Box::new(Node {
                id,
                key: id.into(),
                spec: self.spec.clone(),
                tick: self.ticks[id].clone(),
                children: Rc::new(Children(children)),
            }) as Box<dyn View>;
            let parent = match self.spec.topology {
                Topology::Flat => None,
                Topology::Wide => (id > 0).then_some(0),
                Topology::Balanced => id.checked_sub(1).map(|id| id / 8),
                Topology::Deep => id.checked_sub(1),
            };
            if let Some(parent) = parent {
                descendants[parent].push(node);
            } else {
                roots.push(node);
            }
        }
        roots.reverse();
        if self.spec.topology == Topology::Flat {
            Box::new(Flow(Children(roots)))
        } else {
            Box::new(VStack::new(Children(roots)).alignment(Alignment::Leading))
        }
    }
}

impl View for Document {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(self.clone(), Self::body))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        if self.spec.workload == Workload::StructuralUpdate {
            vec![&self.reduced]
        } else {
            Vec::new()
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The common flat scenario requires wrapping. This small native render object
/// only arranges children; text, surfaces, state, reconciliation and painting
/// stay in ScarletUI. No virtualization or application-side drawing shortcuts.
#[derive(Clone)]
struct Flow(Children);

impl View for Flow {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children(
            self.clone(),
            |_| FlowLayout(Size::ZERO),
            |view| view.0.clone_views(),
        ))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct FlowLayout(Size);

impl ElementRenderObject for FlowLayout {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn render(&mut self) {}
    fn paint<'a>(&'a self, _: &mut scarlet_ui::renderer::PaintContext<'a>, _: Point) -> bool {
        true
    }
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        self.0 = constraints.constrain(Size::ZERO);
        self.0
    }
    fn size(&self) -> Size {
        self.0
    }
    fn layout_with_children(
        &mut self,
        constraints: LayoutConstraints,
        children: &mut [Box<dyn Element>],
    ) -> Size {
        let (mut x, mut y, mut row_height, mut width) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for child in children {
            let size = child.layout(LayoutConstraints::new(
                0.0,
                constraints.max_width,
                0.0,
                f32::INFINITY,
            ));
            if x > 0.0 && x + size.width > constraints.max_width {
                x = 0.0;
                y += row_height + 4.0;
                row_height = 0.0;
            }
            child.set_position(Point::new(x, y));
            width = width.max(x + size.width);
            x += size.width + 4.0;
            row_height = row_height.max(size.height);
        }
        self.0 = constraints.constrain(Size::new(width, y + row_height));
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui::{element::ElementId, pipeline::RenderingPipeline};
    use std::collections::BTreeMap;

    fn nodes(element: &dyn Element, result: &mut BTreeMap<usize, ElementId>) {
        if let Some(node) = element.as_any().downcast_ref::<ComponentElement<Node>>() {
            assert!(result.insert(node.view().id, element.id()).is_none());
        }
        for child in element.children() {
            nodes(child.as_ref(), result);
        }
    }

    fn mounted_nodes(pipeline: &RenderingPipeline) -> BTreeMap<usize, ElementId> {
        let mut result = BTreeMap::new();
        nodes(pipeline.element_tree().root().unwrap(), &mut result);
        result
    }

    fn pipeline(document: &Document) -> RenderingPipeline {
        scarlet_ui::graphics::set_default_font(super::super::FONT).unwrap();
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_root(
            Window::new("test", document.clone())
                .decorated(false)
                .size(Size::new(600.0, 400.0))
                .create_element(),
        );
        pipeline.layout_initial();
        pixels(&mut pipeline);
        pipeline
    }

    fn pixels(pipeline: &mut RenderingPipeline) -> Vec<u8> {
        match pipeline.render_for_present().unwrap() {
            scarlet_ui::renderer::PresentedFrame::Cpu { buffer, .. } => buffer.data().to_vec(),
            _ => panic!("expected a rendered CPU frame"),
        }
    }

    #[test]
    fn structural_changes_remove_real_nodes_and_retain_survivor_identity() {
        for topology in [
            Topology::Flat,
            Topology::Wide,
            Topology::Balanced,
            Topology::Deep,
        ] {
            let mut spec = bench_core::scenarios(bench_core::Preset::Smoke).remove(0);
            spec.workload = Workload::StructuralUpdate;
            spec.nodes = 16;
            spec.topology = topology;
            let document = Document::new(spec);
            let mut pipeline = pipeline(&document);
            let before = mounted_nodes(&pipeline);
            assert_eq!(before.len(), 16, "{topology:?}");
            document.stimulate(1);
            pixels(&mut pipeline);
            let removed = mounted_nodes(&pipeline);
            assert_eq!(removed.len(), 15, "{topology:?}");
            for (id, element) in &removed {
                assert_eq!(before.get(id), Some(element));
            }
            document.stimulate(2);
            pixels(&mut pipeline);
            let restored = mounted_nodes(&pipeline);
            assert_eq!(restored.len(), 16, "{topology:?}");
            for (id, element) in &removed {
                assert_eq!(restored.get(id), Some(element));
            }
            pipeline.teardown();
        }
    }

    #[test]
    fn reactive_updates_change_rendered_pixels_without_remounting_nodes() {
        for workload in [
            Workload::PaintUpdate,
            Workload::LayoutUpdate,
            Workload::TextUpdate,
        ] {
            let mut spec = bench_core::scenarios(bench_core::Preset::Smoke).remove(0);
            spec.workload = workload;
            spec.nodes = 8;
            spec.update_fraction = 0.25;
            let document = Document::new(spec);
            let mut pipeline = pipeline(&document);
            let before_ids = mounted_nodes(&pipeline);
            pipeline.request_redraw();
            let before = pixels(&mut pipeline);
            document.stimulate(1);
            let after = pixels(&mut pipeline);
            assert!(
                before != after,
                "rendered pixels did not change for {workload:?}"
            );
            assert_eq!(mounted_nodes(&pipeline), before_ids, "{workload:?}");
            pipeline.teardown();
        }
    }
}

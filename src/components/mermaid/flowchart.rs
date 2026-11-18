use std::collections::BTreeMap;

use mermaid_rs_renderer::{Direction, Layout, NodeShape, layout::flowchart_quality_metrics};

use super::overlapping_edge_length;

type Point = (f32, f32);

#[derive(Clone, Copy)]
struct Rect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Rect {
    fn around((x, y): Point, half_width: f32, half_height: f32) -> Self {
        Self {
            left: x - half_width,
            top: y - half_height,
            right: x + half_width,
            bottom: y + half_height,
        }
    }

    fn overlaps(self, other: Self) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    fn intersects_segment(self, [start, end]: [Point; 2]) -> bool {
        // Clip the segment's parametric interval against each axis of the rectangle.
        let (mut enter, mut exit) = (0_f32, 1_f32);
        for (from, to, min, max) in [
            (start.0, end.0, self.left, self.right),
            (start.1, end.1, self.top, self.bottom),
        ] {
            let delta = to - from;
            if delta.abs() < 0.001 {
                if from < min || from > max {
                    return false;
                }
                continue;
            }
            let a = (min - from) / delta;
            let b = (max - from) / delta;
            enter = enter.max(a.min(b));
            exit = exit.min(a.max(b));
        }
        enter <= exit
    }

    fn intersects_path(self, points: &[Point]) -> bool {
        points
            .array_windows()
            .any(|&segment| self.intersects_segment(segment))
    }
}

pub(super) fn polish(layout: &mut Layout) {
    // Cluster borders and captions also constrain routing. Keep the renderer's
    // routes and label positions when clusters are involved.
    if !layout.subgraphs.is_empty() {
        return;
    }
    let nodes: Vec<_> = layout
        .nodes
        .values()
        .filter(|node| !node.hidden && node.anchor_subgraph.is_none())
        .map(|node| {
            (
                node.id.clone(),
                Rect {
                    left: node.x - 2.,
                    top: node.y - 2.,
                    right: node.x + node.width + 2.,
                    bottom: node.y + node.height + 2.,
                },
            )
        })
        .collect();

    let endpoint_labels: Vec<_> = layout
        .edges
        .iter()
        .flat_map(|edge| {
            [
                edge.start_label.as_ref().zip(edge.start_label_anchor),
                edge.end_label.as_ref().zip(edge.end_label_anchor),
            ]
            .into_iter()
            .flatten()
            .map(|(label, anchor)| {
                Rect::around(anchor, label.width * 0.5 + 3.4, label.height * 0.5 + 1.8)
            })
        })
        .collect();
    let mut moved = shorten_routes(layout, &nodes, &endpoint_labels);
    let bundles = parallel_pairs(layout);
    separate_parallel_arrows(layout, &bundles, &endpoint_labels, &mut moved);
    separate_parallel_lanes(layout, &bundles, &endpoint_labels, &mut moved);
    reposition_labels(layout, &nodes, &endpoint_labels, &moved);
}

fn parallel_pairs(layout: &Layout) -> Vec<[usize; 2]> {
    let mut bundles = BTreeMap::new();
    for (index, edge) in layout.edges.iter().enumerate() {
        if edge.arrow_end && edge.from != edge.to {
            bundles
                .entry((&edge.from, &edge.to))
                .or_insert_with(Vec::new)
                .push(index);
        }
    }
    bundles
        .into_values()
        .filter_map(|indices| indices.as_slice().try_into().ok())
        .collect()
}

fn separate_parallel_lanes(
    layout: &mut Layout,
    bundles: &[[usize; 2]],
    endpoint_labels: &[Rect],
    moved: &mut [bool],
) {
    const LANE_GAP: f32 = 22.;
    const MIN_ALONGSIDE_LENGTH: f32 = 36.;
    for &[first, second] in bundles {
        let [_, a_start, a_end, _] = layout.edges[first].points.as_slice() else {
            continue;
        };
        let [_, b_start, b_end, _] = layout.edges[second].points.as_slice() else {
            continue;
        };
        let vertical = (a_end.1 - a_start.1).abs() >= (a_end.0 - a_start.0).abs();
        // Convert to coordinates along and across the shared routing direction.
        let axes = |(x, y): Point| if vertical { (y, x) } else { (x, y) };
        let ((a0, a_side), (a1, a_tip_side)) = (axes(*a_start), axes(*a_end));
        let ((b0, b_side), (b1, b_tip_side)) = (axes(*b_start), axes(*b_end));
        let overlap = a0.max(a1).min(b0.max(b1)) - a0.min(a1).max(b0.min(b1));
        let start_gap = b_side - a_side;
        let end_gap = b_tip_side - a_tip_side;
        if overlap < MIN_ALONGSIDE_LENGTH
            || (a_tip_side - a_side).abs() > 4.
            || (b_tip_side - b_side).abs() > 4.
            || start_gap.abs().min(end_gap.abs()) >= LANE_GAP
        {
            continue;
        }
        let order = (start_gap + end_gap).signum();
        let shift = order * (LANE_GAP - (order * start_gap).min(order * end_gap));
        let original = flowchart_quality_metrics(layout).expect("flowchart layout");
        let original_overlap = overlapping_edge_length(layout);
        // Either route can give up its current lane, as long as both stubs still face out.
        for (index, delta) in [(second, shift), (first, -shift)] {
            let old = &layout.edges[index].points;
            let &[start, exit, approach, end] = old.as_slice() else {
                unreachable!()
            };
            let (start_along, start_side) = axes(start);
            let (exit_along, exit_side) = axes(exit);
            let (approach_along, approach_side) = axes(approach);
            let (end_along, end_side) = axes(end);
            let preserves_stub =
                |old: f32, new: f32| old.signum() == new.signum() && new.abs() >= old.abs().min(6.);
            if (start_along - exit_along).abs() > 0.1
                || (approach_along - end_along).abs() > 0.1
                || !preserves_stub(exit_side - start_side, exit_side + delta - start_side)
                || !preserves_stub(approach_side - end_side, approach_side + delta - end_side)
            {
                continue;
            }
            let points = [
                start,
                axes((exit_along, exit_side + delta)),
                axes((approach_along, approach_side + delta)),
                end,
            ];
            if points[1..3]
                .iter()
                .any(|&(x, y)| x < 4. || x > layout.width - 4. || y < 4. || y > layout.height - 4.)
                || endpoint_labels
                    .iter()
                    .any(|rect| rect.intersects_path(&points))
            {
                continue;
            }
            let previous = std::mem::replace(&mut layout.edges[index].points, points.to_vec());
            let candidate = flowchart_quality_metrics(layout).expect("flowchart layout");
            if candidate.geometry_debt_count() <= original.geometry_debt_count()
                && candidate.hard_violation_count() <= original.hard_violation_count()
                && candidate.crossings <= original.crossings
                && overlapping_edge_length(layout) <= original_overlap + 0.1
            {
                moved[index] = true;
                break;
            }
            layout.edges[index].points = previous;
        }
    }
}

pub(super) fn compact_ranks(layout: &mut Layout, direction: Direction, spacing: f32) {
    if !layout.subgraphs.is_empty() || layout.nodes.len() < 5 {
        return;
    }
    let horizontal = matches!(direction, Direction::LeftRight | Direction::RightLeft);
    let axis = |(x, y): Point| if horizontal { x } else { y };
    let mut occupied: Vec<_> = layout
        .nodes
        .values()
        .map(|node| {
            let start = axis((node.x, node.y));
            (start - 4., start + axis((node.width, node.height)) + 4.)
        })
        .collect();
    for edge in &layout.edges {
        for (label, anchor) in [
            (&edge.label, edge.label_anchor),
            (&edge.start_label, edge.start_label_anchor),
            (&edge.end_label, edge.end_label_anchor),
        ] {
            if let (Some(label), Some(anchor)) = (label, anchor) {
                let center = axis(anchor);
                let radius = axis((label.width, label.height)) / 2. + 6.;
                occupied.push((center - radius, center + radius));
            }
        }
    }
    occupied.sort_by(|a, b| a.0.total_cmp(&b.0));
    let gap = spacing.max(44.);
    let mut end = occupied[0].1;
    let mut bands = Vec::new();
    for (next, next_end) in occupied.into_iter().skip(1) {
        if next - end > gap + 12. {
            bands.push((end, next, next - end - gap));
        }
        end = end.max(next_end);
    }
    if bands.is_empty() {
        return;
    }

    // Compress empty bands while translating each node and label as a whole.
    let compress = |position: f32| {
        position
            - bands
                .iter()
                .map(|&(from, to, removed)| {
                    removed * ((position - from) / (to - from)).clamp(0., 1.)
                })
                .sum::<f32>()
    };
    let move_point = |(x, y): Point| {
        if horizontal {
            (compress(x), y)
        } else {
            (x, compress(y))
        }
    };
    let mut candidate = layout.clone();
    for node in candidate.nodes.values_mut() {
        (node.x, node.y) = move_point((node.x, node.y));
    }
    for edge in &mut candidate.edges {
        edge.points
            .iter_mut()
            .for_each(|point| *point = move_point(*point));
        edge.label_anchor = edge.label_anchor.map(move_point);
        edge.start_label_anchor = edge.start_label_anchor.map(move_point);
        edge.end_label_anchor = edge.end_label_anchor.map(move_point);
    }
    (candidate.width, candidate.height) = move_point((candidate.width, candidate.height));
    let original = flowchart_quality_metrics(layout).expect("flowchart layout");
    let compressed = flowchart_quality_metrics(&candidate).expect("flowchart layout");
    if compressed.geometry_debt_count() <= original.geometry_debt_count()
        && compressed.hard_violation_count() <= original.hard_violation_count()
        && compressed.crossings <= original.crossings
        && overlapping_edge_length(&candidate) <= overlapping_edge_length(layout) + 0.1
    {
        *layout = candidate;
    }
}

fn separate_parallel_arrows(
    layout: &mut Layout,
    bundles: &[[usize; 2]],
    endpoint_labels: &[Rect],
    moved: &mut [bool],
) {
    const PORT_GAP: f32 = 14.;
    const CORNER_MARGIN: f32 = 9.;
    for [first, second] in bundles {
        let Some(node) = layout.nodes.get(&layout.edges[*first].to) else {
            continue;
        };
        if !matches!(node.shape, NodeShape::Rectangle | NodeShape::RoundRect) {
            continue;
        }
        let first_route = &layout.edges[*first].points;
        let second_route = &layout.edges[*second].points;
        let [_, .., first_approach, first_tip] = first_route.as_slice() else {
            continue;
        };
        let [_, .., second_approach, second_tip] = second_route.as_slice() else {
            continue;
        };
        // Both arrowheads must approach the same flat face perpendicularly.
        let horizontal = (first_approach.1 - first_tip.1).abs() < 0.1;
        let axes = |(x, y): Point| if horizontal { (x, y) } else { (y, x) };
        let (face, a) = axes(*first_tip);
        let (other_face, b) = axes(*second_tip);
        let (_, a_approach) = axes(*first_approach);
        let (_, b_approach) = axes(*second_approach);
        if (face - other_face).abs() > 0.1
            || (a - a_approach).abs() > 0.1
            || (b - b_approach).abs() > 0.1
            || (a - b).abs() >= PORT_GAP
        {
            continue;
        }
        let (face_min, port_min) = axes((node.x, node.y));
        let (face_max, port_max) = axes((node.x + node.width, node.y + node.height));
        if ((face - face_min).abs() > 1.5 && (face - face_max).abs() > 1.5)
            || port_max - port_min < 2. * CORNER_MARGIN + PORT_GAP
        {
            continue;
        }
        let center = ((a + b) * 0.5).clamp(
            port_min + CORNER_MARGIN + PORT_GAP / 2.,
            port_max - CORNER_MARGIN - PORT_GAP / 2.,
        );
        let ports = [center - PORT_GAP / 2., center + PORT_GAP / 2.];
        let original = flowchart_quality_metrics(layout).expect("flowchart layout");
        let overlap = overlapping_edge_length(layout);
        let original_routes = [first_route.clone(), second_route.clone()];
        let mut best = None;
        for [left, right] in [ports, [ports[1], ports[0]]] {
            for (index, (old, port)) in [*first, *second]
                .into_iter()
                .zip(original_routes.iter().zip([left, right]))
            {
                let mut route = old.clone();
                let [.., approach, tip] = route.as_mut_slice() else {
                    unreachable!()
                };
                if horizontal {
                    approach.1 = port;
                    tip.1 = port;
                } else {
                    approach.0 = port;
                    tip.0 = port;
                }
                layout.edges[index].points = route;
            }
            let candidate = flowchart_quality_metrics(layout).expect("flowchart layout");
            if candidate.geometry_debt_count() <= original.geometry_debt_count()
                && candidate.hard_violation_count() <= original.hard_violation_count()
                && candidate.crossings <= original.crossings
                && overlapping_edge_length(layout) <= overlap + 0.1
                && [*first, *second].into_iter().all(|index| {
                    !endpoint_labels
                        .iter()
                        .any(|rect| rect.intersects_path(&layout.edges[index].points))
                })
                && best
                    .as_ref()
                    .is_none_or(|(cost, _)| candidate.quality_score < *cost)
            {
                best = Some((
                    candidate.quality_score,
                    [
                        layout.edges[*first].points.clone(),
                        layout.edges[*second].points.clone(),
                    ],
                ));
            }
        }
        let (changed, routes) = match best {
            Some((_, routes)) => (true, routes),
            None => (false, original_routes),
        };
        for (index, route) in [*first, *second].into_iter().zip(routes) {
            layout.edges[index].points = route;
            moved[index] |= changed;
        }
    }
}

fn shorten_routes(
    layout: &mut Layout,
    nodes: &[(String, Rect)],
    endpoint_labels: &[Rect],
) -> Vec<bool> {
    let mut shortcuts = Vec::new();
    for (index, edge) in layout.edges.iter().enumerate() {
        let &[start, exit, .., approach, end] = edge.points.as_slice() else {
            continue;
        };
        if edge.from == edge.to {
            continue;
        }
        let existing_length: f32 = edge
            .points
            .array_windows()
            .map(|&[a, b]| distance(a, b))
            .sum();
        let mut consider = |points: &[Point], segment| {
            let saving = existing_length
                - points
                    .array_windows()
                    .map(|&[a, b]| distance(a, b))
                    .sum::<f32>();
            if saving > 24.
                && saving > existing_length * 0.2
                && !nodes.iter().any(|(id, rect)| {
                    *id != edge.from && *id != edge.to && rect.intersects_segment(segment)
                })
                && !endpoint_labels
                    .iter()
                    .any(|rect| rect.intersects_segment(segment))
            {
                shortcuts.push((index, saving, points.to_vec()));
            }
        };
        if (start.0 - end.0).abs() < 0.1 || (start.1 - end.1).abs() < 0.1 {
            consider(&[start, end], [start, end]);
        }
        if edge.points.len() > 4 {
            consider(&[start, exit, approach, end], [exit, approach]);
        }
    }
    let mut moved = vec![false; layout.edges.len()];
    if shortcuts.is_empty() {
        return moved;
    }
    shortcuts.sort_by(|left, right| right.1.total_cmp(&left.1));
    let mut original = flowchart_quality_metrics(layout).expect("flowchart layout");
    let mut overlap = overlapping_edge_length(layout);
    for (index, _, points) in shortcuts.into_iter().take(8) {
        if moved[index] {
            continue;
        }
        let old_points = std::mem::replace(&mut layout.edges[index].points, points);
        let candidate = flowchart_quality_metrics(layout).expect("flowchart layout");
        let candidate_overlap = overlapping_edge_length(layout);
        if candidate.geometry_debt_count() > original.geometry_debt_count()
            || candidate.hard_violation_count() > original.hard_violation_count()
            || candidate.crossings > original.crossings
            || candidate_overlap > overlap + 0.1
        {
            layout.edges[index].points = old_points;
        } else {
            original = candidate;
            overlap = candidate_overlap;
            moved[index] = true;
        }
    }
    moved
}

fn reposition_labels(
    layout: &mut Layout,
    nodes: &[(String, Rect)],
    endpoint_labels: &[Rect],
    moved: &[bool],
) {
    let label_bounds: Vec<_> = layout
        .edges
        .iter()
        .map(|edge| {
            edge.label
                .as_ref()
                .zip(edge.label_anchor)
                .map(|(label, anchor)| {
                    // Match the padding used for flowchart center labels by the SVG renderer.
                    (anchor, label.width * 0.5 + 4.5, label.height * 0.5 + 2.2)
                })
        })
        .collect();
    let mut occupied: Vec<_> = label_bounds
        .iter()
        .map(|label| label.map(|(anchor, width, height)| Rect::around(anchor, width, height)))
        .collect();
    for (index, label) in label_bounds.into_iter().enumerate() {
        let Some((anchor, width, height)) = label else {
            continue;
        };
        let rect = Rect::around(anchor, width, height);
        if !moved[index]
            && !nodes.iter().any(|(_, node)| node.overlaps(rect))
            && !layout
                .edges
                .iter()
                .zip(moved)
                .any(|(edge, moved)| *moved && rect.intersects_path(&edge.points))
        {
            continue;
        }
        occupied[index] = None;
        let edge = &layout.edges[index];
        let mut best = None;
        for &[start, end] in edge.points.array_windows() {
            let length = distance(start, end);
            if length < 8. {
                continue;
            }
            let normal = ((start.1 - end.1) / length, (end.0 - start.0) / length);
            let clearance = normal.0.abs() * width + normal.1.abs() * height + 6.;
            for sample in 1..16 {
                let fraction = sample as f32 / 16.;
                for side in [-1., 1.] {
                    for extra in [0., 12., 36., 64.] {
                        let offset = side * (clearance + extra);
                        let point = (
                            start.0 + (end.0 - start.0) * fraction + normal.0 * offset,
                            start.1 + (end.1 - start.1) * fraction + normal.1 * offset,
                        );
                        let candidate = Rect::around(point, width, height);
                        let cost = distance(anchor, point) + extra;
                        if best.is_some_and(|(best_cost, _)| best_cost <= cost)
                            || candidate.left < 0.
                            || candidate.top < 0.
                            || candidate.right > layout.width
                            || candidate.bottom > layout.height
                            || nodes
                                .iter()
                                .map(|(_, node)| node)
                                .chain(occupied.iter().flatten())
                                .chain(endpoint_labels)
                                .any(|other| other.overlaps(candidate))
                            || layout
                                .edges
                                .iter()
                                .any(|edge| candidate.intersects_path(&edge.points))
                        {
                            continue;
                        }
                        best = Some((cost, point));
                    }
                }
            }
        }
        if let Some((_, point)) = best {
            layout.edges[index].label_anchor = Some(point);
            occupied[index] = Some(Rect::around(point, width, height));
        } else {
            occupied[index] = Some(rect);
        }
    }
}

fn distance(a: Point, b: Point) -> f32 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mermaid_rs_renderer::{RenderOptions, compute_layout, parse_mermaid_strict};

    #[test]
    fn spreads_long_parallel_runs_with_distinct_ports() {
        let graph = parse_mermaid_strict("flowchart LR\n A --> B\n A -.-> B")
            .unwrap()
            .graph;
        let options = RenderOptions::default();
        let mut layout = compute_layout(&graph, &options.theme, &options.layout);
        let target = layout.nodes.get_mut("B").unwrap();
        target.y += 160.;
        let (right, target_y) = (target.x + target.width, target.y + target.height / 2.);
        let source = layout.nodes.get_mut("A").unwrap();
        source.x = right + 100.;
        layout.width = source.x + source.width + 8.;
        layout.height += 160.;
        let source_y = source.y + source.height / 2.;
        for (index, edge) in layout.edges.iter_mut().enumerate() {
            let offset = index as f32 * 14.;
            let lane = right + 10. + index as f32 * 5.;
            edge.points = vec![
                (source.x, source_y + offset),
                (lane, source_y + offset),
                (lane, target_y + offset),
                (right, target_y + offset),
            ];
        }
        let tips = [(right, target_y), (right, target_y + 14.)];
        let mut moved = [false; 2];

        separate_parallel_lanes(&mut layout, &[[0, 1]], &[], &mut moved);

        let first = &layout.edges[0].points;
        let second = &layout.edges[1].points;
        assert!(moved.into_iter().any(|changed| changed));
        assert!((first[1].0 - second[1].0).abs() >= 21.9);
        assert!((first[2].0 - second[2].0).abs() >= 21.9);
        assert_eq!([first[3], second[3]], tips);
        let metrics = flowchart_quality_metrics(&layout).unwrap();
        assert_eq!(metrics.crossings, 0);
        assert_eq!(metrics.geometry_debt_count(), 0);
    }

    #[test]
    fn separates_parallel_arrowheads_on_the_same_face() {
        let graph = parse_mermaid_strict("flowchart LR\n A --> B\n A -.-> B")
            .unwrap()
            .graph;
        let options = RenderOptions::default();
        let mut layout = compute_layout(&graph, &options.theme, &options.layout);
        let source = &layout.nodes["A"];
        let target = &layout.nodes["B"];
        let x = target.x;
        let y = target.y + target.height / 2.;
        let starts = [
            (source.x + source.width, source.y + source.height / 2. - 10.),
            (source.x + source.width, source.y + source.height / 2. + 10.),
        ];
        for (index, (edge, start)) in layout.edges.iter_mut().zip(starts).enumerate() {
            let port_y = y + index as f32 * 2.;
            edge.points = vec![start, (x - 10., port_y), (x, port_y)];
        }

        let mut moved = [false; 2];
        separate_parallel_arrows(&mut layout, &[[0, 1]], &[], &mut moved);

        assert_eq!(moved, [true; 2]);
        let left = &layout.edges[0].points;
        let right = &layout.edges[1].points;
        assert_eq!(left[0], starts[0]);
        assert_eq!(right[0], starts[1]);
        assert!((left[2].1 - right[2].1).abs() >= 13.9);
        let metrics = flowchart_quality_metrics(&layout).unwrap();
        assert_eq!(metrics.crossings, 0);
        assert_eq!(metrics.geometry_debt_count(), 0);
    }

    #[test]
    fn compacts_empty_rank_bands_without_resizing_nodes() {
        let graph = parse_mermaid_strict("flowchart TD\n A --> B --> C --> D --> E")
            .unwrap()
            .graph;
        let mut options = RenderOptions::default();
        options.layout.rank_spacing = 120.;
        options.layout.flowchart.auto_spacing.enabled = false;
        let mut layout = compute_layout(&graph, &options.theme, &options.layout);
        let (height, node_height) = (layout.height, layout.nodes["B"].height);

        compact_ranks(&mut layout, graph.direction, 60.);

        assert!(layout.height < height);
        assert_eq!(layout.nodes["B"].height, node_height);
        assert_eq!(
            flowchart_quality_metrics(&layout)
                .unwrap()
                .geometry_debt_count(),
            0
        );
    }

    #[test]
    fn shortens_detour_and_moves_label_off_node() {
        let graph = parse_mermaid_strict("flowchart LR\n A[Start] -->|Next| B[End]")
            .unwrap()
            .graph;
        let mut options = RenderOptions::default();
        options.layout.fast_text_metrics = true;
        let mut layout = compute_layout(&graph, &options.theme, &options.layout);
        layout.height += 100.;

        let edge = &mut layout.edges[0];
        let start = edge.points[0];
        let end = *edge.points.last().unwrap();
        let exit = (start.0 + 8., start.1);
        let approach = (end.0 - 8., end.1);
        edge.points = vec![
            start,
            exit,
            (exit.0, start.1 + 80.),
            (approach.0, start.1 + 80.),
            approach,
            end,
        ];
        edge.label_anchor = Some((layout.nodes["A"].x + 30., start.1));
        let before = flowchart_quality_metrics(&layout).unwrap();

        polish(&mut layout);

        let after = flowchart_quality_metrics(&layout).unwrap();
        let edge = &layout.edges[0];
        assert!(after.path_length < before.path_length);
        assert_eq!(edge.points.len(), 2);
        assert_eq!(after.geometry_debt_count(), 0);
        assert_eq!(edge.points.first(), Some(&start));
        assert_eq!(edge.points.last(), Some(&end));
        let label = edge.label.as_ref().unwrap();
        let label = Rect::around(
            edge.label_anchor.unwrap(),
            label.width * 0.5 + 4.5,
            label.height * 0.5 + 2.2,
        );
        assert!(!label.intersects_path(&edge.points));
        assert!(layout.nodes.values().all(|node| !label.overlaps(Rect {
            left: node.x,
            top: node.y,
            right: node.x + node.width,
            bottom: node.y + node.height,
        })));
    }
}

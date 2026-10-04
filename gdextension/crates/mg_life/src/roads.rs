//! LifeGen stage 5: roads.
//!
//! Ported from Randlebrot's `rb_world::lifegen::roads`. Which settlements are
//! linked is decided on straight-line distance; each link is then routed over
//! the navigation grid with A*, so roads bend around hard terrain and never
//! cross open water. A link with no land route is dropped.

use crate::settlements::{Settlement, SizeClass};
use rayon::prelude::*;
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

// Distances are Randlebrot's, converted from cells at 8 per world unit.
/// A capital may get one extra link to its nearest capital within this distance.
const EXTRA_CAPITAL_LINK_MAX_WU: f64 = 187.5;
/// Towns and larger link to nearby towns and larger within this distance...
const TOWN_LINK_MAX_WU: f64 = 37.5;
/// ...up to this many extra links each.
const TOWN_EXTRA_LINKS: usize = 2;
/// A highway is routed through towns lying this close to its straight line...
const WAYPOINT_CORRIDOR_WU: f64 = 25.0;
/// ...that are at least this far from either end...
const WAYPOINT_END_MARGIN_WU: f64 = 3.75;
/// ...and this far from each other along the line.
const WAYPOINT_MIN_SPACING_WU: f64 = 7.5;
/// Links longer than this are not built.
const MAX_ROAD_LENGTH_WU: f64 = 250.0;
/// Routes may stray this far outside the box around their two ends.
const SEARCH_PADDING_WU: f64 = 25.0;
/// Route points closer than this to the simplified line are dropped.
const SIMPLIFY_TOLERANCE_WU: f64 = 0.375;
const MAX_SEARCH_EXPANSIONS: usize = 2_000_000;
/// Passable land is never harder to cross than this.
const MIN_NAVIGATION_EASE: f32 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RoadKind {
    /// Between two faction capitals.
    Highway,
    /// To or from a capital or city.
    Road,
    Trail,
}

impl RoadKind {
    pub const ALL: [Self; 3] = [Self::Highway, Self::Road, Self::Trail];
}

#[derive(Debug, Clone, PartialEq)]
pub struct Road {
    /// Settlement ids at either end.
    pub from_settlement: u32,
    pub to_settlement: u32,
    pub kind: RoadKind,
    /// Simplified route, as cells from one end to the other.
    pub path: Vec<(usize, usize)>,
    /// Travel cost of the full route: distance in cells weighted by terrain.
    pub cost: f32,
}

/// Build the road network. `navigation_cost` is the stage 1 grid: 0.0
/// impassable, 1.0 trivial. Deterministic; uses no seed.
pub fn build_roads(
    settlements: &[Settlement],
    navigation_cost: &[f32],
    width: usize,
    height: usize,
    cells_per_world_unit: f64,
) -> Vec<Road> {
    if settlements.len() < 2 {
        return Vec::new();
    }
    let links = choose_links(settlements, cells_per_world_unit);
    let padding = (SEARCH_PADDING_WU * cells_per_world_unit).ceil() as usize;
    let tolerance = SIMPLIFY_TOLERANCE_WU * cells_per_world_unit;

    links
        .par_iter()
        .filter_map(|&(a, b, kind)| {
            let (from, to) = (&settlements[a], &settlements[b]);
            if distance(from.position, to.position) > MAX_ROAD_LENGTH_WU * cells_per_world_unit {
                return None;
            }
            let bounds = SearchBounds::around(from.position, to.position, padding, width, height);
            let (path, cost) =
                find_route(from.position, to.position, navigation_cost, width, bounds)?;
            Some(Road {
                from_settlement: from.id,
                to_settlement: to.id,
                kind,
                path: simplify_path(&path, tolerance),
                cost,
            })
        })
        .collect()
}

fn distance(a: (usize, usize), b: (usize, usize)) -> f64 {
    let (dx, dy) = (a.0 as f64 - b.0 as f64, a.1 as f64 - b.1 as f64);
    (dx * dx + dy * dy).sqrt()
}

fn is_town_or_larger(size_class: SizeClass) -> bool {
    matches!(
        size_class,
        SizeClass::Metropolis | SizeClass::City | SizeClass::Town
    )
}

fn road_kind(a: &Settlement, b: &Settlement) -> RoadKind {
    let is_capital = |s: &Settlement| s.size_class == SizeClass::Metropolis;
    let is_city = |s: &Settlement| s.size_class == SizeClass::City;
    if is_capital(a) && is_capital(b) {
        RoadKind::Highway
    } else if is_capital(a) || is_capital(b) || is_city(a) || is_city(b) {
        RoadKind::Road
    } else {
        RoadKind::Trail
    }
}

/// Decide which pairs of settlements get a road, as indices into
/// `settlements`:
/// 1. a minimum spanning tree over all settlements, so everything connects;
/// 2. a spanning tree over capitals, plus one extra nearby capital each;
/// 3. up to two extra links from each town or larger to nearby ones;
/// 4. highways split to pass through towns along their way.
fn choose_links(
    settlements: &[Settlement],
    cells_per_world_unit: f64,
) -> Vec<(usize, usize, RoadKind)> {
    let positions: Vec<(usize, usize)> = settlements.iter().map(|s| s.position).collect();
    let mut links: Vec<(usize, usize)> = Vec::new();
    let mut linked: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut add_link = |links: &mut Vec<(usize, usize)>, a: usize, b: usize| {
        if a != b && linked.insert((a.min(b), a.max(b))) {
            links.push((a, b));
        }
    };

    for (a, b) in minimum_spanning_tree(&positions) {
        add_link(&mut links, a, b);
    }

    let capitals: Vec<usize> = (0..settlements.len())
        .filter(|&i| settlements[i].size_class == SizeClass::Metropolis)
        .collect();
    let capital_positions: Vec<(usize, usize)> = capitals.iter().map(|&i| positions[i]).collect();
    for (a, b) in minimum_spanning_tree(&capital_positions) {
        add_link(&mut links, capitals[a], capitals[b]);
    }
    let already_linked: BTreeSet<(usize, usize)> =
        links.iter().map(|&(a, b)| (a.min(b), a.max(b))).collect();
    for &capital in &capitals {
        let nearest_unlinked = capitals
            .iter()
            .filter(|&&other| {
                other != capital
                    && !already_linked.contains(&(capital.min(other), capital.max(other)))
                    && distance(positions[capital], positions[other])
                        <= EXTRA_CAPITAL_LINK_MAX_WU * cells_per_world_unit
            })
            .min_by(|&&a, &&b| {
                distance(positions[capital], positions[a])
                    .total_cmp(&distance(positions[capital], positions[b]))
            });
        if let Some(&other) = nearest_unlinked {
            add_link(&mut links, capital, other);
        }
    }

    let towns: Vec<usize> = (0..settlements.len())
        .filter(|&i| is_town_or_larger(settlements[i].size_class))
        .collect();
    let already_linked: BTreeSet<(usize, usize)> =
        links.iter().map(|&(a, b)| (a.min(b), a.max(b))).collect();
    for &town in &towns {
        let mut nearby: Vec<(f64, usize)> = towns
            .iter()
            .filter(|&&other| {
                other != town && !already_linked.contains(&(town.min(other), town.max(other)))
            })
            .map(|&other| (distance(positions[town], positions[other]), other))
            .filter(|&(d, _)| d <= TOWN_LINK_MAX_WU * cells_per_world_unit)
            .collect();
        nearby.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for &(_, other) in nearby.iter().take(TOWN_EXTRA_LINKS) {
            add_link(&mut links, town, other);
        }
    }

    let mut kinded = Vec::with_capacity(links.len());
    for (a, b) in links {
        let kind = road_kind(&settlements[a], &settlements[b]);
        if kind != RoadKind::Highway {
            kinded.push((a, b, kind));
            continue;
        }
        let mut chain = vec![a];
        chain.extend(highway_waypoints(a, b, settlements, cells_per_world_unit));
        chain.push(b);
        for pair in chain.windows(2) {
            kinded.push((pair[0], pair[1], RoadKind::Highway));
        }
    }
    kinded
}

/// Prim's algorithm on straight-line distance. Returns index pairs.
fn minimum_spanning_tree(positions: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let count = positions.len();
    if count < 2 {
        return Vec::new();
    }
    let mut in_tree = vec![false; count];
    let mut best_distance = vec![f64::MAX; count];
    let mut best_parent = vec![0usize; count];
    let mut edges = Vec::with_capacity(count - 1);

    let mut newest = 0;
    in_tree[0] = true;
    for _ in 1..count {
        for i in 0..count {
            if in_tree[i] {
                continue;
            }
            let d = distance(positions[newest], positions[i]);
            if d < best_distance[i] {
                best_distance[i] = d;
                best_parent[i] = newest;
            }
        }
        let next = (0..count)
            .filter(|&i| !in_tree[i])
            .min_by(|&a, &b| best_distance[a].total_cmp(&best_distance[b]))
            .expect("a settlement is still outside the tree");
        in_tree[next] = true;
        edges.push((best_parent[next], next));
        newest = next;
    }
    edges
}

/// Towns or larger near the straight line from `a` to `b`, in order along it.
/// A highway is routed through them instead of running end to end.
fn highway_waypoints(
    a: usize,
    b: usize,
    settlements: &[Settlement],
    cells_per_world_unit: f64,
) -> Vec<usize> {
    let (start, end) = (settlements[a].position, settlements[b].position);
    let length = distance(start, end);
    if length < 1.0 {
        return Vec::new();
    }
    let direction = (
        (end.0 as f64 - start.0 as f64) / length,
        (end.1 as f64 - start.1 as f64) / length,
    );
    let margin = WAYPOINT_END_MARGIN_WU * cells_per_world_unit;

    let mut along_line: Vec<(f64, usize)> = settlements
        .iter()
        .enumerate()
        .filter(|&(index, settlement)| {
            index != a && index != b && is_town_or_larger(settlement.size_class)
        })
        .filter_map(|(index, settlement)| {
            let offset = (
                settlement.position.0 as f64 - start.0 as f64,
                settlement.position.1 as f64 - start.1 as f64,
            );
            let along = offset.0 * direction.0 + offset.1 * direction.1;
            let across = (offset.1 * direction.0 - offset.0 * direction.1).abs();
            let on_the_way = along >= margin
                && along <= length - margin
                && across <= WAYPOINT_CORRIDOR_WU * cells_per_world_unit;
            on_the_way.then_some((along, index))
        })
        .collect();
    along_line.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));

    let mut waypoints: Vec<(f64, usize)> = Vec::new();
    for (along, index) in along_line {
        let spaced = waypoints.last().is_none_or(|&(previous, _)| {
            along - previous >= WAYPOINT_MIN_SPACING_WU * cells_per_world_unit
        });
        if spaced {
            waypoints.push((along, index));
        }
    }
    waypoints.into_iter().map(|(_, index)| index).collect()
}

/// The part of the grid a route search may use.
#[derive(Clone, Copy)]
struct SearchBounds {
    min_x: usize,
    min_y: usize,
    width: usize,
    height: usize,
}

impl SearchBounds {
    fn around(
        a: (usize, usize),
        b: (usize, usize),
        padding: usize,
        grid_width: usize,
        grid_height: usize,
    ) -> Self {
        let min_x = a.0.min(b.0).saturating_sub(padding);
        let min_y = a.1.min(b.1).saturating_sub(padding);
        let max_x = (a.0.max(b.0) + padding).min(grid_width - 1);
        let max_y = (a.1.max(b.1) + padding).min(grid_height - 1);
        Self {
            min_x,
            min_y,
            width: max_x - min_x + 1,
            height: max_y - min_y + 1,
        }
    }

    fn local_index(&self, x: usize, y: usize) -> usize {
        (y - self.min_y) * self.width + (x - self.min_x)
    }

    fn contains(&self, x: i64, y: i64) -> bool {
        x >= self.min_x as i64
            && y >= self.min_y as i64
            && x < (self.min_x + self.width) as i64
            && y < (self.min_y + self.height) as i64
    }
}

/// Cheapest route between two cells (A*, 8-connected). Stepping onto a cell
/// costs the step length divided by its navigation ease; cells with ease 0.0
/// cannot be entered. Returns the cells of the route and its total cost, or
/// `None` if there is no route inside `bounds`.
fn find_route(
    from: (usize, usize),
    to: (usize, usize),
    navigation_cost: &[f32],
    grid_width: usize,
    bounds: SearchBounds,
) -> Option<(Vec<(usize, usize)>, f32)> {
    const STEPS: [(i64, i64, f32); 8] = [
        (-1, 0, 1.0),
        (1, 0, 1.0),
        (0, -1, 1.0),
        (0, 1, 1.0),
        (-1, -1, 1.414),
        (1, -1, 1.414),
        (-1, 1, 1.414),
        (1, 1, 1.414),
    ];
    const NO_PARENT: u32 = u32::MAX;
    if from == to {
        return Some((vec![from], 0.0));
    }

    let cell_count = bounds.width * bounds.height;
    let mut best_cost = vec![f32::MAX; cell_count];
    let mut parent = vec![NO_PARENT; cell_count];
    // Straight-line distance never overestimates: no step costs less than its length.
    let estimate = |x: usize, y: usize| distance((x, y), to) as f32;
    let start = bounds.local_index(from.0, from.1);
    let goal = bounds.local_index(to.0, to.1);

    // (estimated total cost in hundredths, local cell index)
    let mut open: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
    best_cost[start] = 0.0;
    open.push(Reverse((
        (estimate(from.0, from.1) * 100.0) as u32,
        start as u32,
    )));

    let mut expansions = 0;
    while let Some(Reverse((_, current))) = open.pop() {
        let current = current as usize;
        if current == goal {
            let mut path = Vec::new();
            let mut cell = current;
            loop {
                path.push((
                    bounds.min_x + cell % bounds.width,
                    bounds.min_y + cell / bounds.width,
                ));
                if parent[cell] == NO_PARENT {
                    break;
                }
                cell = parent[cell] as usize;
            }
            path.reverse();
            return Some((path, best_cost[goal]));
        }
        expansions += 1;
        if expansions > MAX_SEARCH_EXPANSIONS {
            return None;
        }

        let x = bounds.min_x + current % bounds.width;
        let y = bounds.min_y + current / bounds.width;
        for &(dx, dy, step_length) in &STEPS {
            let (nx, ny) = (x as i64 + dx, y as i64 + dy);
            if !bounds.contains(nx, ny) {
                continue;
            }
            let (nx, ny) = (nx as usize, ny as usize);
            let ease = navigation_cost[ny * grid_width + nx];
            if ease == 0.0 {
                continue;
            }
            let cost = best_cost[current] + step_length / ease.max(MIN_NAVIGATION_EASE);
            let neighbour = bounds.local_index(nx, ny);
            if cost < best_cost[neighbour] {
                best_cost[neighbour] = cost;
                parent[neighbour] = current as u32;
                open.push(Reverse((
                    ((cost + estimate(nx, ny)) * 100.0) as u32,
                    neighbour as u32,
                )));
            }
        }
    }
    None
}

/// Douglas-Peucker: drop route points that lie within `tolerance` cells of
/// the line through the points kept. The two ends are always kept.
fn simplify_path(path: &[(usize, usize)], tolerance: f64) -> Vec<(usize, usize)> {
    if path.len() <= 2 {
        return path.to_vec();
    }
    let (first, last) = (path[0], path[path.len() - 1]);
    let (farthest, farthest_distance) = path[1..path.len() - 1]
        .iter()
        .enumerate()
        .map(|(index, &point)| (index + 1, distance_to_line(point, first, last)))
        .fold((0, 0.0), |best, candidate| {
            if candidate.1 > best.1 {
                candidate
            } else {
                best
            }
        });
    if farthest_distance <= tolerance {
        return vec![first, last];
    }
    let mut simplified = simplify_path(&path[..=farthest], tolerance);
    simplified.pop();
    simplified.extend(simplify_path(&path[farthest..], tolerance));
    simplified
}

fn distance_to_line(point: (usize, usize), a: (usize, usize), b: (usize, usize)) -> f64 {
    let length = distance(a, b);
    if length < 1e-6 {
        return distance(point, a);
    }
    let (ax, ay) = (a.0 as f64, a.1 as f64);
    let (bx, by) = (b.0 as f64, b.1 as f64);
    let (px, py) = (point.0 as f64, point.1 as f64);
    ((by - ay) * px - (bx - ax) * py + bx * ay - by * ax).abs() / length
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settlement(id: u32, position: (usize, usize), size_class: SizeClass) -> Settlement {
        Settlement {
            id,
            position,
            province_id: 1,
            size_class,
        }
    }

    /// Open ground with a wall of water down column `wall_x`, except at `gap_y`.
    fn grid_with_wall(
        width: usize,
        height: usize,
        wall_x: usize,
        gap_y: Option<usize>,
    ) -> Vec<f32> {
        let mut grid = vec![1.0; width * height];
        for y in 0..height {
            if Some(y) != gap_y {
                grid[y * width + wall_x] = 0.0;
            }
        }
        grid
    }

    #[test]
    fn a_road_joins_its_two_settlements() {
        let settlements = [
            settlement(1, (2, 5), SizeClass::Village),
            settlement(2, (17, 5), SizeClass::Village),
        ];
        let roads = build_roads(&settlements, &vec![1.0; 20 * 10], 20, 10, 1.0);

        assert_eq!(roads.len(), 1);
        assert_eq!(roads[0].path.first(), Some(&(2, 5)));
        assert_eq!(roads[0].path.last(), Some(&(17, 5)));
        assert_eq!(roads[0].kind, RoadKind::Trail);
    }

    #[test]
    fn a_route_goes_through_the_gap_in_a_wall_of_water() {
        let grid = grid_with_wall(20, 20, 10, Some(18));
        let bounds = SearchBounds::around((2, 2), (17, 2), 25, 20, 20);

        let (path, cost) = find_route((2, 2), (17, 2), &grid, 20, bounds).expect("a route exists");

        assert!(path.contains(&(10, 18)));
        assert!(path.iter().all(|&(x, y)| grid[y * 20 + x] > 0.0));
        assert!(cost > 15.0);
    }

    #[test]
    fn no_road_is_built_across_water() {
        let settlements = [
            settlement(1, (2, 5), SizeClass::Village),
            settlement(2, (17, 5), SizeClass::Village),
        ];
        let grid = grid_with_wall(20, 10, 10, None);

        assert!(build_roads(&settlements, &grid, 20, 10, 1.0).is_empty());
    }

    #[test]
    fn every_settlement_is_connected_on_open_ground() {
        let settlements: Vec<Settlement> = (0..12)
            .map(|i| {
                settlement(
                    i + 1,
                    ((i as usize * 7) % 38 + 1, (i as usize * 5) % 18 + 1),
                    SizeClass::Village,
                )
            })
            .collect();
        let roads = build_roads(&settlements, &vec![1.0; 40 * 20], 40, 20, 1.0);

        let mut reached = BTreeSet::from([1u32]);
        loop {
            let before = reached.len();
            for road in &roads {
                if reached.contains(&road.from_settlement) || reached.contains(&road.to_settlement)
                {
                    reached.insert(road.from_settlement);
                    reached.insert(road.to_settlement);
                }
            }
            if reached.len() == before {
                break;
            }
        }
        assert_eq!(reached.len(), settlements.len());
    }

    #[test]
    fn road_kind_follows_the_larger_settlement() {
        let capital = settlement(1, (0, 0), SizeClass::Metropolis);
        let city = settlement(2, (0, 0), SizeClass::City);
        let village = settlement(3, (0, 0), SizeClass::Village);

        assert_eq!(road_kind(&capital, &capital), RoadKind::Highway);
        assert_eq!(road_kind(&capital, &village), RoadKind::Road);
        assert_eq!(road_kind(&city, &village), RoadKind::Road);
        assert_eq!(road_kind(&village, &village), RoadKind::Trail);
    }

    #[test]
    fn a_highway_passes_through_a_town_on_its_way() {
        let settlements = [
            settlement(1, (0, 10), SizeClass::Metropolis),
            settlement(2, (60, 10), SizeClass::Metropolis),
            settlement(3, (30, 12), SizeClass::Town),
            settlement(4, (30, 60), SizeClass::Town),
        ];

        assert_eq!(highway_waypoints(0, 1, &settlements, 1.0), vec![2]);
    }

    #[test]
    fn simplifying_a_straight_route_keeps_only_its_ends() {
        let straight: Vec<(usize, usize)> = (0..10).map(|x| (x, 3)).collect();
        let bent = [(0, 0), (5, 0), (5, 5)];

        assert_eq!(simplify_path(&straight, 0.4), vec![(0, 3), (9, 3)]);
        assert_eq!(simplify_path(&bent, 0.4), bent.to_vec());
    }
}

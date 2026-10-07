//! LifeGen stage 5: roads.
//!
//! Ported from Randlebrot's `rb_world::lifegen::roads`. Which settlements are
//! linked is decided on straight-line distance; each link is then routed over
//! the navigation grid with A*, so roads bend around hard terrain and never
//! cross open water. A link with no land route is dropped.
//!
//! On a ring, links and routes take the short way round, across the seam if
//! that is nearer.

use crate::grid::Grid;
use crate::settlements::{Settlement, SizeClass};
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};

// Distances are Randlebrot's, converted from cells at 8 per world unit.
/// A capital may get one extra link to its nearest capital within this distance.
const EXTRA_CAPITAL_LINK_MAX_WU: f64 = 187.5;
/// Towns and larger link to nearby towns and larger within this distance...
const TOWN_LINK_MAX_WU: f64 = 37.5;
/// ...up to this many extra links each.
const TOWN_EXTRA_LINKS: usize = 1;
/// Ground that already carries a road is at least this easy to travel, so
/// later roads run along earlier ones and merge into a network instead of
/// each taking its own line.
const ROAD_EASE: f32 = 0.95;
/// Crossing a river costs this much more than the ground would, over and
/// above the river's own hard going, so roads cross at few points.
const RIVER_CROSSING_COST: f32 = 6.0;
/// A highway is routed through towns lying this close to its straight line...
const WAYPOINT_CORRIDOR_WU: f64 = 25.0;
/// ...that are at least this far from either end...
const WAYPOINT_END_MARGIN_WU: f64 = 3.75;
/// ...and this far from each other along the line.
const WAYPOINT_MIN_SPACING_WU: f64 = 7.5;
/// Links longer than this are not built.
const MAX_ROAD_LENGTH_WU: f64 = 250.0;
/// A route may be this much longer than the straight way between its ends.
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
    /// Simplified route, as cells from one end to the other. On a ring,
    /// consecutive cells may sit on opposite edges of the grid: the road
    /// crosses the seam between them.
    pub path: Vec<(usize, usize)>,
    /// Travel cost of the full route: distance in cells weighted by terrain.
    pub cost: f32,
    /// Cells where the road crosses a river: a bridge or a ford at each.
    pub crossings: Vec<(usize, usize)>,
}

/// Build the road network. `navigation_cost` is the stage 1 grid: 0.0
/// impassable, 1.0 trivial; `river_distance` is its distance to the nearest
/// river, in cells. Deterministic; uses no seed.
///
/// Roads are routed one at a time, highways first, and ground that already
/// carries a road is easy, so later roads join earlier ones: the result is
/// a network of trunks and branches, not a tangle of separate lines.
pub fn build_roads(
    settlements: &[Settlement],
    navigation_cost: &[f32],
    river_distance: &[f32],
    width: usize,
    height: usize,
    grid: &Grid,
) -> Vec<Road> {
    if settlements.len() < 2 {
        return Vec::new();
    }
    let mut links = choose_links(settlements, grid);
    links.sort_by_key(|&(a, b, kind)| (kind, a, b));
    let padding = SEARCH_PADDING_WU * grid.cells_per_world_unit;
    let tolerance = SIMPLIFY_TOLERANCE_WU * grid.cells_per_world_unit;
    let is_river = |cell: usize| river_distance[cell] < 1.0;
    // A river is hard to cross, and harder still away from a road already
    // crossing it. The ease grid is updated as roads are built.
    let mut ease: Vec<f32> = navigation_cost
        .iter()
        .enumerate()
        .map(|(cell, &ease)| {
            if is_river(cell) {
                ease / RIVER_CROSSING_COST
            } else {
                ease
            }
        })
        .collect();

    let mut search = RouteSearch::new(width, height);
    let mut roads = Vec::with_capacity(links.len());
    for (a, b, kind) in links {
        let (from, to) = (&settlements[a], &settlements[b]);
        if grid.distance(from.position, to.position)
            > MAX_ROAD_LENGTH_WU * grid.cells_per_world_unit
        {
            continue;
        }
        let Some((path, cost)) = search.find_route(from.position, to.position, &ease, padding, grid)
        else {
            continue;
        };
        let mut crossings = Vec::new();
        for &cell in &path {
            let index = cell.1 * width + cell.0;
            if is_river(index) && navigation_cost[index] > 0.0 {
                crossings.push(cell);
            }
            if navigation_cost[index] > 0.0 {
                ease[index] = ease[index].max(ROAD_EASE);
            }
        }
        roads.push(Road {
            from_settlement: from.id,
            to_settlement: to.id,
            kind,
            path: simplify_path(&path, tolerance, grid),
            cost,
            crossings,
        });
    }
    roads
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
/// 3. one extra link from each town or larger to the nearest other;
/// 4. highways split to pass through towns along their way.
fn choose_links(settlements: &[Settlement], grid: &Grid) -> Vec<(usize, usize, RoadKind)> {
    let positions: Vec<(usize, usize)> = settlements.iter().map(|s| s.position).collect();
    let mut links: Vec<(usize, usize)> = Vec::new();
    let mut linked: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut add_link = |links: &mut Vec<(usize, usize)>, a: usize, b: usize| {
        if a != b && linked.insert((a.min(b), a.max(b))) {
            links.push((a, b));
        }
    };

    for (a, b) in minimum_spanning_tree(&positions, grid) {
        add_link(&mut links, a, b);
    }

    let capitals: Vec<usize> = (0..settlements.len())
        .filter(|&i| settlements[i].size_class == SizeClass::Metropolis)
        .collect();
    let capital_positions: Vec<(usize, usize)> = capitals.iter().map(|&i| positions[i]).collect();
    for (a, b) in minimum_spanning_tree(&capital_positions, grid) {
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
                    && grid.distance(positions[capital], positions[other])
                        <= EXTRA_CAPITAL_LINK_MAX_WU * grid.cells_per_world_unit
            })
            .min_by(|&&a, &&b| {
                grid.distance(positions[capital], positions[a])
                    .total_cmp(&grid.distance(positions[capital], positions[b]))
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
            .map(|&other| (grid.distance(positions[town], positions[other]), other))
            .filter(|&(d, _)| d <= TOWN_LINK_MAX_WU * grid.cells_per_world_unit)
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
        chain.extend(highway_waypoints(a, b, settlements, grid));
        chain.push(b);
        for pair in chain.windows(2) {
            kinded.push((pair[0], pair[1], RoadKind::Highway));
        }
    }
    kinded
}

/// Prim's algorithm on straight-line distance. Returns index pairs.
fn minimum_spanning_tree(positions: &[(usize, usize)], grid: &Grid) -> Vec<(usize, usize)> {
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
            let d = grid.distance(positions[newest], positions[i]);
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

/// Where a point lies relative to the line from `a` to `b`: how far along
/// it (from `a`, in cells) and how far off it. From the three distances
/// alone, so it holds on any shape of grid.
fn along_and_across(grid: &Grid, point: (usize, usize), a: (usize, usize), b: (usize, usize)) -> (f64, f64) {
    let length = grid.distance(a, b);
    let (to_a, to_b) = (grid.distance(point, a), grid.distance(point, b));
    if length < 1e-6 {
        return (0.0, to_a);
    }
    let along = (to_a * to_a - to_b * to_b + length * length) / (2.0 * length);
    let across = (to_a * to_a - along * along).max(0.0).sqrt();
    (along, across)
}

/// Towns or larger near the straight line from `a` to `b`, in order along it.
/// A highway is routed through them instead of running end to end.
fn highway_waypoints(a: usize, b: usize, settlements: &[Settlement], grid: &Grid) -> Vec<usize> {
    let (start, end) = (settlements[a].position, settlements[b].position);
    let length = grid.distance(start, end);
    if length < 1.0 {
        return Vec::new();
    }
    let margin = WAYPOINT_END_MARGIN_WU * grid.cells_per_world_unit;

    let mut along_line: Vec<(f64, usize)> = settlements
        .iter()
        .enumerate()
        .filter(|&(index, settlement)| {
            index != a && index != b && is_town_or_larger(settlement.size_class)
        })
        .filter_map(|(index, settlement)| {
            let (along, across) = along_and_across(grid, settlement.position, start, end);
            let on_the_way = along >= margin
                && along <= length - margin
                && across <= WAYPOINT_CORRIDOR_WU * grid.cells_per_world_unit;
            on_the_way.then_some((along, index))
        })
        .collect();
    along_line.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));

    let mut waypoints: Vec<(f64, usize)> = Vec::new();
    for (along, index) in along_line {
        let spaced = waypoints.last().is_none_or(|&(previous, _)| {
            along - previous >= WAYPOINT_MIN_SPACING_WU * grid.cells_per_world_unit
        });
        if spaced {
            waypoints.push((along, index));
        }
    }
    waypoints.into_iter().map(|(_, index)| index).collect()
}

/// The working memory of route searches: one entry per cell of the grid,
/// kept between searches and reset only where a search touched it.
struct RouteSearch {
    width: usize,
    height: usize,
    best_cost: Vec<f32>,
    parent: Vec<u32>,
    touched: Vec<u32>,
}

const NO_PARENT: u32 = u32::MAX;

impl RouteSearch {
    fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            best_cost: vec![f32::MAX; width * height],
            parent: vec![NO_PARENT; width * height],
            touched: Vec::new(),
        }
    }

    /// Cheapest route between two cells (A* over the grid's eight-connected
    /// cells). Stepping onto a cell costs the step length divided by its
    /// navigation ease; cells with ease 0.0 cannot be entered. The search
    /// keeps to cells no more than `padding` cells off the straight way
    /// between the ends. Returns the cells of the route and its total cost,
    /// or `None` if there is no route.
    fn find_route(
        &mut self,
        from: (usize, usize),
        to: (usize, usize),
        navigation_cost: &[f32],
        padding: f64,
        grid: &Grid,
    ) -> Option<(Vec<(usize, usize)>, f32)> {
        for &cell in &self.touched {
            self.best_cost[cell as usize] = f32::MAX;
            self.parent[cell as usize] = NO_PARENT;
        }
        self.touched.clear();
        if from == to {
            return Some((vec![from], 0.0));
        }
        let (width, height) = (self.width, self.height);
        let index = |cell: (usize, usize)| cell.1 * width + cell.0;
        let cell_of = |index: usize| (index % width, index / width);
        let length = grid.distance(from, to);
        // A route may be longer than the straight way by the padding and no
        // more, so a cell is out of bounds if the way through it would be.
        let within_bounds =
            |cell: (usize, usize)| grid.distance(from, cell) + grid.distance(cell, to) <= length + padding;
        // Straight-line distance never overestimates: no step costs less than its length.
        let estimate = |cell: (usize, usize)| grid.distance(cell, to) as f32;

        let (start, goal) = (index(from), index(to));
        // (estimated total cost in hundredths, cell index)
        let mut open: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
        self.best_cost[start] = 0.0;
        self.touched.push(start as u32);
        open.push(Reverse(((estimate(from) * 100.0) as u32, start as u32)));

        let mut expansions = 0;
        while let Some(Reverse((_, current))) = open.pop() {
            let current = current as usize;
            if current == goal {
                let mut path = Vec::new();
                let mut cell = current;
                loop {
                    path.push(cell_of(cell));
                    if self.parent[cell] == NO_PARENT {
                        break;
                    }
                    cell = self.parent[cell] as usize;
                }
                path.reverse();
                return Some((path, self.best_cost[goal]));
            }
            expansions += 1;
            if expansions > MAX_SEARCH_EXPANSIONS {
                return None;
            }

            let (x, y) = cell_of(current);
            for (next, step_length) in grid.neighbours(x, y, width, height) {
                let neighbour = index(next);
                let ease = navigation_cost[neighbour];
                if ease == 0.0 || !within_bounds(next) {
                    continue;
                }
                let cost =
                    self.best_cost[current] + step_length as f32 / ease.max(MIN_NAVIGATION_EASE);
                if cost < self.best_cost[neighbour] {
                    if self.best_cost[neighbour] == f32::MAX {
                        self.touched.push(neighbour as u32);
                    }
                    self.best_cost[neighbour] = cost;
                    self.parent[neighbour] = current as u32;
                    open.push(Reverse((
                        ((cost + estimate(next)) * 100.0) as u32,
                        neighbour as u32,
                    )));
                }
            }
        }
        None
    }
}

/// Douglas-Peucker: drop route points that lie within `tolerance` cells of
/// the line through the points kept. The two ends are always kept.
fn simplify_path(path: &[(usize, usize)], tolerance: f64, grid: &Grid) -> Vec<(usize, usize)> {
    if path.len() <= 2 {
        return path.to_vec();
    }
    let (first, last) = (path[0], path[path.len() - 1]);
    let (farthest, farthest_distance) = path[1..path.len() - 1]
        .iter()
        .enumerate()
        .map(|(index, &point)| (index + 1, along_and_across(grid, point, first, last).1))
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
    let mut simplified = simplify_path(&path[..=farthest], tolerance, grid);
    simplified.pop();
    simplified.extend(simplify_path(&path[farthest..], tolerance, grid));
    simplified
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAT: Grid = Grid::flat(1.0);

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
        let mut cells = vec![1.0; width * height];
        for y in 0..height {
            if Some(y) != gap_y {
                cells[y * width + wall_x] = 0.0;
            }
        }
        cells
    }

    #[test]
    fn a_road_joins_its_two_settlements() {
        let settlements = [
            settlement(1, (2, 5), SizeClass::Village),
            settlement(2, (17, 5), SizeClass::Village),
        ];
        let roads = build_roads(
            &settlements,
            &vec![1.0; 20 * 10],
            &vec![f32::MAX; 20 * 10],
            20,
            10,
            &FLAT,
        );

        assert_eq!(roads.len(), 1);
        assert_eq!(roads[0].path.first(), Some(&(2, 5)));
        assert_eq!(roads[0].path.last(), Some(&(17, 5)));
        assert_eq!(roads[0].kind, RoadKind::Trail);
        assert!((roads[0].cost - 15.0).abs() < 0.01);
    }

    #[test]
    fn on_a_ring_a_road_takes_the_short_way_across_the_seam() {
        let settlements = [
            settlement(1, (2, 5), SizeClass::Village),
            settlement(2, (97, 5), SizeClass::Village),
        ];
        let open_ground = vec![1.0; 100 * 10];

        let flat = build_roads(
            &settlements,
            &open_ground,
            &vec![f32::MAX; 100 * 10],
            100,
            10,
            &FLAT,
        );
        let ring = build_roads(
            &settlements,
            &open_ground,
            &vec![f32::MAX; 100 * 10],
            100,
            10,
            &Grid::sphere(1.0, 100, 10),
        );

        assert!((flat[0].cost - 95.0).abs() < 0.01);
        // Steps along a row near the equator are a little under a cell.
        assert!((ring[0].cost - 5.0).abs() < 0.1);
        assert_eq!(ring[0].path.first(), Some(&(2, 5)));
        assert_eq!(ring[0].path.last(), Some(&(97, 5)));
    }

    #[test]
    fn on_a_ring_water_along_the_seam_still_blocks_the_road() {
        // Water down column 0 with no gap. The short way is blocked and the long
        // way round is far outside the search box, so no road is built.
        let settlements = [
            settlement(1, (2, 5), SizeClass::Village),
            settlement(2, (97, 5), SizeClass::Village),
        ];
        let cells = grid_with_wall(100, 10, 0, None);

        assert!(build_roads(
            &settlements,
            &cells,
            &vec![f32::MAX; 100 * 10],
            100,
            10,
            &Grid::sphere(1.0, 100, 10)
        )
        .is_empty());
    }

    #[test]
    fn a_route_goes_through_the_gap_in_a_wall_of_water() {
        let cells = grid_with_wall(20, 20, 10, Some(18));
        let mut search = RouteSearch::new(20, 20);

        let (path, cost) = search
            .find_route((2, 2), (17, 2), &cells, 25.0, &FLAT)
            .expect("a route exists");

        assert!(path.contains(&(10, 18)));
        assert!(path.iter().all(|&(x, y)| cells[y * 20 + x] > 0.0));
        assert!(cost > 15.0);
    }

    #[test]
    fn no_road_is_built_across_water() {
        let settlements = [
            settlement(1, (2, 5), SizeClass::Village),
            settlement(2, (17, 5), SizeClass::Village),
        ];
        let cells = grid_with_wall(20, 10, 10, None);

        assert!(
            build_roads(&settlements, &cells, &vec![f32::MAX; 20 * 10], 20, 10, &FLAT).is_empty()
        );
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
        let roads = build_roads(
            &settlements,
            &vec![1.0; 40 * 20],
            &vec![f32::MAX; 40 * 20],
            40,
            20,
            &FLAT,
        );

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

        assert_eq!(highway_waypoints(0, 1, &settlements, &FLAT), vec![2]);
    }

    #[test]
    fn on_a_ring_highway_waypoints_are_found_across_the_seam() {
        // Capitals 40 apart across the seam of a 200-wide world, on its
        // equator, a town between them.
        let settlements = [
            settlement(1, (180, 10), SizeClass::Metropolis),
            settlement(2, (20, 10), SizeClass::Metropolis),
            settlement(3, (195, 11), SizeClass::Town),
            settlement(4, (100, 10), SizeClass::Town),
        ];

        assert_eq!(
            highway_waypoints(0, 1, &settlements, &Grid::sphere(1.0, 200, 20)),
            vec![2]
        );
    }

    #[test]
    fn simplifying_a_straight_route_keeps_only_its_ends() {
        let straight: Vec<(usize, usize)> = (0..10).map(|x| (x, 3)).collect();
        let bent = [(0, 0), (5, 0), (5, 5)];

        assert_eq!(simplify_path(&straight, 0.4, &FLAT), vec![(0, 3), (9, 3)]);
        assert_eq!(simplify_path(&bent, 0.4, &FLAT), bent.to_vec());
    }

    #[test]
    fn later_roads_run_along_earlier_ones_and_rivers_are_crossed_where_a_road_is() {
        // Two villages south of a town, across a river running along row 5.
        let settlements = vec![
            settlement(1, (10, 2), SizeClass::Town),
            settlement(2, (8, 8), SizeClass::Village),
            settlement(3, (12, 8), SizeClass::Village),
        ];
        let (width, height) = (20, 10);
        let river_distance: Vec<f32> = (0..width * height)
            .map(|cell: usize| (cell / width).abs_diff(5) as f32)
            .collect();

        let roads = build_roads(
            &settlements,
            &vec![1.0; width * height],
            &river_distance,
            width,
            height,
            &FLAT,
        );

        assert!(!roads.is_empty());
        let crossings: std::collections::BTreeSet<(usize, usize)> = roads
            .iter()
            .flat_map(|road| road.crossings.iter().copied())
            .collect();
        // Both villages reach the town over the river, by one crossing: the
        // second road to cross followed the first.
        assert!(
            roads
                .iter()
                .filter(|road| !road.crossings.is_empty())
                .count()
                >= 1
        );
        assert_eq!(crossings.len(), 1, "{crossings:?}");
    }
}

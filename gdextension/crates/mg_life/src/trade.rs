//! LifeGen stage 6: trade.
//!
//! Trade flows up the settlement hierarchy: every settlement sends its trade
//! to the nearest strictly larger settlement it can reach by road. Because
//! flows only go from smaller to larger, they form a directed acyclic graph.
//!
//! Randlebrot's version was a stub that treated any two settlements near any
//! road as connected. This one follows the road network.

use crate::grid::Grid;
use crate::provinces::ProvinceMap;
use crate::roads::Road;
use crate::settlements::{Settlement, SizeClass};

#[derive(Debug, Clone, PartialEq)]
pub struct TradeFlow {
    /// Settlement ids. Trade moves from the smaller to the larger.
    pub from_settlement: u32,
    pub to_settlement: u32,
    /// Mean habitability of the source settlement's province.
    pub value: f32,
}

/// Higher is larger.
fn size_rank(size_class: SizeClass) -> u8 {
    match size_class {
        SizeClass::Ruins => 0,
        SizeClass::Outpost => 1,
        SizeClass::Village => 2,
        SizeClass::Town => 3,
        SizeClass::City => 4,
        SizeClass::Metropolis => 5,
    }
}

/// For each settlement (by index), a label shared by exactly the settlements
/// it can reach by road. Union-find over the roads.
fn road_networks(settlements: &[Settlement], roads: &[Road]) -> Vec<usize> {
    fn root(parents: &mut [usize], mut node: usize) -> usize {
        while parents[node] != node {
            parents[node] = parents[parents[node]];
            node = parents[node];
        }
        node
    }
    // Settlement ids are 1-based and dense, so id - 1 is the index.
    let mut parents: Vec<usize> = (0..settlements.len()).collect();
    for road in roads {
        let a = root(&mut parents, (road.from_settlement - 1) as usize);
        let b = root(&mut parents, (road.to_settlement - 1) as usize);
        parents[a.max(b)] = a.min(b);
    }
    (0..settlements.len())
        .map(|index| root(&mut parents, index))
        .collect()
}

/// Build the trade flows. Deterministic; uses no seed.
pub fn build_trade_flows(
    settlements: &[Settlement],
    roads: &[Road],
    province_map: &ProvinceMap,
    grid: Grid,
) -> Vec<TradeFlow> {
    let networks = road_networks(settlements, roads);
    let distance_squared = |a: &Settlement, b: &Settlement| {
        let distance = grid.distance(a.position, b.position);
        distance * distance
    };

    settlements
        .iter()
        .enumerate()
        .filter_map(|(index, source)| {
            let value = province_map.provinces[(source.province_id - 1) as usize].habitability;
            if value <= 0.0 {
                return None;
            }
            let market = settlements
                .iter()
                .enumerate()
                .filter(|&(other, candidate)| {
                    networks[other] == networks[index]
                        && size_rank(candidate.size_class) > size_rank(source.size_class)
                })
                .min_by(|a, b| {
                    distance_squared(source, a.1)
                        .total_cmp(&distance_squared(source, b.1))
                        .then(a.1.id.cmp(&b.1.id))
                })?
                .1;
            Some(TradeFlow {
                from_settlement: source.id,
                to_settlement: market.id,
                value,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provinces::Province;
    use crate::roads::RoadKind;
    use mg_core::TileType;

    fn settlement(id: u32, x: usize, size_class: SizeClass) -> Settlement {
        Settlement {
            id,
            position: (x, 0),
            province_id: 1,
            size_class,
        }
    }

    fn road(from_settlement: u32, to_settlement: u32) -> Road {
        Road {
            from_settlement,
            to_settlement,
            kind: RoadKind::Trail,
            path: vec![],
            cost: 1.0,
            crossings: vec![],
        }
    }

    fn one_province(habitability: f32) -> ProvinceMap {
        ProvinceMap {
            width: 0,
            height: 0,
            province_ids: vec![],
            provinces: vec![Province {
                id: 1,
                site: (0, 0),
                biome: TileType::Plains,
                habitability,
                area_cells: 100,
                is_coastal: false,
                is_river_junction: false,
                elevation_mean: 0.1,
                terrain_cost: 0.5,
                light_level: 0.4,
                resources: 0.3,
            }],
            adjacency: vec![vec![], vec![]],
        }
    }

    #[test]
    fn trade_flows_to_the_nearest_larger_settlement_on_the_road_network() {
        // village - town - city in a line, all joined by road
        let settlements = [
            settlement(1, 0, SizeClass::Village),
            settlement(2, 10, SizeClass::Town),
            settlement(3, 30, SizeClass::City),
        ];
        let roads = [road(1, 2), road(2, 3)];

        let flows = build_trade_flows(&settlements, &roads, &one_province(0.6), Grid::flat(1.0));

        assert_eq!(
            flows,
            vec![
                TradeFlow {
                    from_settlement: 1,
                    to_settlement: 2,
                    value: 0.6
                },
                TradeFlow {
                    from_settlement: 2,
                    to_settlement: 3,
                    value: 0.6
                },
            ]
        );
    }

    #[test]
    fn the_largest_settlement_sends_no_trade() {
        let settlements = [
            settlement(1, 0, SizeClass::Metropolis),
            settlement(2, 10, SizeClass::Metropolis),
        ];

        assert!(build_trade_flows(&settlements, &[road(1, 2)], &one_province(0.6), Grid::flat(1.0)).is_empty());
    }

    #[test]
    fn no_trade_without_a_road_between_them() {
        // The city is closer, but only the town is reachable by road.
        let settlements = [
            settlement(1, 0, SizeClass::Village),
            settlement(2, 5, SizeClass::City),
            settlement(3, 20, SizeClass::Town),
        ];

        let flows = build_trade_flows(&settlements, &[road(1, 3)], &one_province(0.6), Grid::flat(1.0));

        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].to_settlement, 3);
    }

    #[test]
    fn road_networks_join_settlements_linked_through_others() {
        let settlements = [
            settlement(1, 0, SizeClass::Village),
            settlement(2, 1, SizeClass::Village),
            settlement(3, 2, SizeClass::Village),
            settlement(4, 3, SizeClass::Village),
        ];

        let networks = road_networks(&settlements, &[road(1, 2), road(2, 3)]);

        assert_eq!(networks[0], networks[2]);
        assert_ne!(networks[0], networks[3]);
    }
}

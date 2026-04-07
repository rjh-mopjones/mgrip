use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TileType {
    // Water
    #[default]
    Sea,
    ShallowSea,
    ContinentalShelf,
    DeepOcean,
    OceanTrench,
    OceanRidge,
    River,

    // Coastal
    Beach,
    Mangrove,
    CoralReef,
    RockyCoast,
    SeaCliff,

    // Frozen (nightside)
    White,
    Glacier,
    Snow,
    IceSheet,
    FrozenBog,
    Tundra,
    Taiga,
    AlpineMeadow,

    // Temperate (terminator zone)
    Plains,
    Meadow,
    Forest,
    DeciduousForest,
    TemperateRainforest,
    Woodland,
    Scrubland,
    Marsh,
    Steppe,
    Mountain,
    Plateau,

    // Warm/subtropical
    SubtropicalForest,
    DryWoodland,
    Thornland,
    HighlandSavanna,
    CloudForest,

    // Hot (dayside)
    Savanna,
    Jungle,
    Desert,
    Sahara,
    Erg,
    Hamada,
    SaltFlat,
    Badlands,
    Oasis,
    Volcanic,
    LavaField,
    MoltenWaste,
    ScorchedRock,
}

impl TileType {
    pub fn rgb(&self) -> [u8; 3] {
        match self {
            Self::Sea => [0, 191, 255],
            Self::ShallowSea => [100, 200, 240],
            Self::ContinentalShelf => [70, 150, 200],
            Self::DeepOcean => [0, 40, 100],
            Self::OceanTrench => [0, 51, 102],
            Self::OceanRidge => [120, 80, 60],
            Self::River => [64, 164, 223],

            Self::Beach => [220, 182, 130],
            Self::Mangrove => [38, 18, 32],         // dark purple-brown coastal
            Self::CoralReef => [200, 100, 120],
            Self::RockyCoast => [98, 95, 108],
            Self::SeaCliff => [135, 135, 155],

            // Nightside ice/frozen — white/blue ice is correct (not vegetation)
            Self::White => [250, 252, 255],
            Self::Glacier => [210, 228, 255],
            Self::Snow => [235, 242, 255],
            Self::IceSheet => [220, 238, 255],
            // Frozen fringe — dark gray-purple (bioluminescent ecosystem, no photosynthesis)
            Self::FrozenBog => [60, 50, 80],
            Self::Tundra => [80, 65, 95],
            Self::Taiga => [28, 18, 38],   // near-black; dark-adapted conifers
            Self::AlpineMeadow => [70, 50, 80],

            // Terminator / temperate — dark photosynthesizers: black, maroon, burgundy
            // RED SUPERGIANT → no blue-green light → no green pigment viable
            Self::Plains => [85, 45, 65],          // sparse dark vegetation on plain
            Self::Meadow => [70, 35, 55],           // deep maroon-purple
            Self::Forest => [30, 10, 25],           // near-black canopy
            Self::DeciduousForest => [50, 20, 40],  // dark burgundy
            Self::TemperateRainforest => [20, 5, 18], // almost black
            Self::Woodland => [60, 25, 45],         // dark burgundy-purple
            Self::Scrubland => [115, 80, 65],       // dry reddish-brown scrub
            Self::Marsh => [45, 30, 55],            // dark swampy purple
            Self::Steppe => [145, 115, 85],         // dry ochre-brown (sparse)
            Self::Mountain => [105, 105, 112],
            Self::Plateau => [130, 75, 55],

            // Warm/subtropical — transitioning to hot; darker vegetation toward dayside
            Self::SubtropicalForest => [40, 15, 30],   // dark burgundy
            Self::DryWoodland => [130, 95, 65],        // dry brown
            Self::Thornland => [140, 100, 65],         // reddish-brown
            Self::HighlandSavanna => [170, 145, 90],   // dry highland tan
            Self::CloudForest => [22, 8, 28],          // near-black cloud canopy

            // Hot/dayside — scorched, no complex vegetation survives
            Self::Savanna => [185, 162, 95],       // dry yellowish-tan (dead analogs)
            Self::Jungle => [22, 5, 18],           // near-black xerophyte jungle
            Self::Desert => [255, 210, 90],
            Self::Sahara => [248, 168, 60],
            Self::Erg => [232, 205, 130],
            Self::Hamada => [130, 92, 68],
            Self::SaltFlat => [238, 232, 215],
            Self::Badlands => [175, 98, 62],
            Self::Oasis => [55, 18, 45],           // dark maroon (oasis plants appear black)
            Self::Volcanic => [64, 28, 28],
            Self::LavaField => [90, 35, 20],
            Self::MoltenWaste => [110, 25, 10],
            Self::ScorchedRock => [58, 52, 48],
        }
    }

    pub fn color(&self) -> [u8; 4] {
        let [r, g, b] = self.rgb();
        [r, g, b, 255]
    }
}

pub type BiomeType = TileType;

extends CanvasLayer
class_name MapOverlay

## Full-screen biome map overlay. Toggle with M.
## Shows the 512×512 biome image scaled to fit the viewport,
## a red dot at the player's block position, and live coordinates.

const MAP_SIZE := 500.0   # display pixels on each axis

var _world_x: float
var _world_y: float

var _bg:          ColorRect
var _map_rect:    TextureRect
var _marker:      ColorRect
var _map_label:   Label   # coords below map
var _title:       Label   # "MAP  [M to close]"

var _hud:         Label   # always-visible coords in top-left (added to separate layer)

func _ready() -> void:
	layer = 10
	visible = false

	_bg = ColorRect.new()
	_bg.color = Color(0.0, 0.0, 0.0, 0.78)
	_bg.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	add_child(_bg)

	_map_rect = TextureRect.new()
	_map_rect.stretch_mode = TextureRect.STRETCH_SCALE
	_map_rect.size = Vector2(MAP_SIZE, MAP_SIZE)
	add_child(_map_rect)

	_marker = ColorRect.new()
	_marker.color = Color(1.0, 0.1, 0.1)
	_marker.size = Vector2(8.0, 8.0)
	add_child(_marker)

	_title = Label.new()
	_title.add_theme_font_size_override("font_size", 16)
	_title.add_theme_color_override("font_color", Color(0.8, 0.8, 0.8))
	_title.text = "BIOME MAP   [M] close"
	add_child(_title)

	_map_label = Label.new()
	_map_label.add_theme_font_size_override("font_size", 15)
	_map_label.add_theme_color_override("font_color", Color(1.0, 1.0, 1.0))
	add_child(_map_label)

## Call once after terrain generation to bake the biome image into a texture.
func setup(biome_map: MgBiomeMap, world_x: float, world_y: float) -> void:
	_world_x = world_x
	_world_y = world_y
	var rgba := biome_map.export_layer_rgba("biome")
	var img  := Image.create_from_data(512, 512, false, Image.FORMAT_RGBA8, rgba)
	_map_rect.texture = ImageTexture.create_from_image(img)

## Call every frame to keep marker + label fresh.
func refresh(player_pos: Vector3) -> void:
	if _hud:
		_hud.text = _coord_text(player_pos)
	if not visible:
		return
	_layout(player_pos)

func toggle() -> void:
	visible = not visible

## Attach a persistent HUD label to a separate CanvasLayer so coords show
## even when the map is closed.
func attach_hud(root: Node) -> void:
	var cl := CanvasLayer.new()
	cl.layer = 5
	_hud = Label.new()
	_hud.position = Vector2(10.0, 10.0)
	_hud.add_theme_font_size_override("font_size", 14)
	_hud.add_theme_color_override("font_color", Color(1.0, 1.0, 1.0))
	_hud.add_theme_color_override("font_shadow_color", Color(0.0, 0.0, 0.0, 0.9))
	_hud.add_theme_constant_override("shadow_offset_x", 1)
	_hud.add_theme_constant_override("shadow_offset_y", 1)
	cl.add_child(_hud)
	root.add_child(cl)

# ── Internals ──────────────────────────────────────────────────────────────────

func _layout(player_pos: Vector3) -> void:
	var vp   := get_viewport().get_visible_rect().size
	var orig := Vector2((vp.x - MAP_SIZE) * 0.5, (vp.y - MAP_SIZE) * 0.5)
	_map_rect.position = orig

	var bx := clampf(player_pos.x, 0.0, 511.0)
	var bz := clampf(player_pos.z, 0.0, 511.0)
	var dot := orig + Vector2(bx / 511.0, bz / 511.0) * MAP_SIZE
	_marker.position = dot - Vector2(4.0, 4.0)

	_title.position    = orig - Vector2(0.0, 24.0)
	_map_label.position = orig + Vector2(0.0, MAP_SIZE + 8.0)
	_map_label.text    = _coord_text(player_pos)

func _coord_text(p: Vector3) -> String:
	var bx := int(p.x)
	var bz := int(p.z)
	var wx := _world_x + p.x / 512.0
	var wz := _world_y + p.z / 512.0
	return "Block (%d, %d)   Y: %d   World (%.3f, %.3f)" % [bx, bz, int(p.y), wx, wz]

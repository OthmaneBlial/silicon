# Freedoom E1M1 gameplay through SILICON

The Phase 70 sample reads `E1M1` from an external Freedoom Phase 1 IWAD. It
builds BSP-leaf floor and ceiling polygons, one-sided walls, and two-sided upper
and lower wall tiers from the WAD's classic map lumps. It palette-decodes the
64×64 floor and ceiling flats and composes wall textures from
`TEXTURE1`/`TEXTURE2`, `PNAMES`, and classic patch columns, using `PLAYPAL` for
both. Two-sided middle textures keep unpainted patch pixels transparent and
render as depth-tested cutouts through the existing SIR discard shader.
BSP cells are split at linedef boundaries before sector-valid pieces supply flat
geometry, including leaves whose seg endpoints do not enclose an area. The
seg-endpoint hull remains a fallback when no BSP piece validates.
Sidedef offsets and the upper/lower or masked-middle pegging flags set wall
UVs. Sector light levels tint the sampled pixels. Geometry, textures,
billboards, transform uniforms, and GLSL SPIR-V shaders are submitted to
SILICON's CPU renderer; no game framebuffer or other renderer is copied.

Download [Freedoom 0.13.0](https://github.com/freedoom/freedoom/releases/tag/v0.13.0)
at upstream commit
[`cfb8644b1a8dc7d7d2177e6a892ccaa2922bdaae`](https://github.com/freedoom/freedoom/commit/cfb8644b1a8dc7d7d2177e6a892ccaa2922bdaae),
extract `freedoom1.wad`, then run:

```sh
cargo run --release --example freedoom_map -- /path/to/freedoom1.wad
cargo run --release --example freedoom_map -- /path/to/freedoom1.wad --interactive
cargo run --release --example freedoom_map -- /path/to/freedoom1.wad --map E1M2 /tmp/freedoom_e1m2.png
```

Map selection defaults to E1M1; `--map` accepts another marker in the WAD, such
as E1M2, for either rendering or interactive play. Each map uses its own
player-1 start and the same SILICON pipeline. Special-11 exits load the next
episode map when its marker exists. Special-51 secret exits load that episode's
M9, whose ordinary exit returns to E1M4, E2M6, E3M7, or E4M3, respectively.
Health, ammo, keys, and armor carry forward while map-local counters reset. Episode-
ending map exits stop at `EXITED` because the prototype has no finale. These
routes follow id Software's
[`G_DoCompleted`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/g_game.c)
and [`P_UseSpecialLine`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_switch.c).
The E1M2 capture below records the
sector-clipped BSP-cell pass with its episode sky: 8,059 triangles across 364
draws and 804 of 1,104 horizontal leaves. The sky texture fills the previously
clear opening near the right edge. Visual completeness beyond E1M2 remains in
progress. The first command writes
`output/freedoom_map.png`. The interactive view uses
WASD to move and strafe, arrow keys to turn, Shift to run, `1` for the pistol,
and `2` for the fist. Space attacks with the selected weapon; a berserk pack
automatically selects the fist. `Q` always punches. Press `E` to open ordinary
doors, operate manual lifts, or use the exit,
and Escape to exit. Every frame submits the scene again through SILICON;
movement stays inside
a BSP-leaf floor, keeps a 16-unit margin from one-sided or explicitly blocking
lines, limits steps to 24 units, and requires 56 units of ceiling clearance.
WAD stim packs, medikits, health bonuses, soul spheres, berserk packs, radiation suits,
invulnerability and partial-invisibility spheres, light-amplification visors,
clips, ammo boxes, green/blue
armor, armor bonuses, and keys render as cutout
billboards. Pickups require clear sight and a 24-unit range. Health/ammo and
armor upgrades stay on the map when they cannot improve the player's inventory;
repeated keys, health bonuses, soul spheres, and armor bonuses are consumed on
contact. Health pickups cap at 100; bonuses and soul spheres can raise health to
200. Armor and pistol ammo cap at 200. Green armor absorbs one third of damage,
blue armor one half, and armor bonuses add one point up to 200. These
rules follow id Software's [`P_GiveBody`, `P_GiveArmor`,
`P_TouchSpecialThing`, and `P_DamageMobj`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_inter.c).
Radiation suits set or refresh a 60-second timer and prevent sector-7 nukage
damage while active, following id Software's [`P_GivePower`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_inter.c)
and [`P_PlayerInSpecialSector`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_spec.c).
The prototype also recognizes map thing 2022 as an invulnerability sphere when
present. Collecting one starts or refreshes a 30-second timer; while active,
enemy melee, hitscan, fireball, and sector damage leave both health and armor
unchanged. The duration and immunity follow id Software's [`P_GivePower` and
`P_DamageMobj`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_inter.c)
and [`INVULNTICS`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/doomdef.h).
Map thing 2024 is a partial-invisibility sphere. Collecting one starts or
refreshes a 60-second timer; while active, former-human and shotgunner hitscan
attacks can miss and imp fireballs can veer off target. Melee attacks remain
accurate. The sample models each hitscan attack as one ray and does not draw
Doom's fuzzy player-shadow effect. The duration and enemy aim deviation follow
id Software's [`P_GivePower`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_inter.c),
[`INVISTICS`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/doomdef.h),
and [`A_FaceTarget`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_enemy.c).
Map thing 2045 is a light-amplification visor. It starts or refreshes a
120-second effect; during its final 128 Doom tics, the light alternates every
eight tics. While lit, SILICON applies a full-bright vertex tint to map
materials, sprites, projectiles, and the weapon. This approximates Doom's almost-full-bright fixed
colormap without reproducing its indexed PLAYPAL colormap tables, following
[`INFRATICS` and `P_GivePower`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/doomdef.h)
and [`P_PlayerThink`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_user.c).
The player starts with
50 pistol rounds. Press `E` within 64 units from the front of a special-1 door
to raise its back sector at 70 units per second, wait 150 tics (about 4.3
seconds), then close it. If the player or a living enemy is in the sector, the
door reverses and opens again. The ceiling geometry and collision update while
it moves. Press `E` from the front of E1M1's one-sided special-11 exit line to
load E1M2 when its WAD marker exists, as described above. A special-51 line
similarly loads the episode's M9 secret map when present.
The WAD's single special-117 use door follows the same wait-and-close behavior
at 280 units per second, four times the normal door speed, as in id Software's
[`EV_VerticalDoor`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_doors.c).
Walking across a special-2 line opens each sector with its tag and leaves the
door open. The release WAD has six such lines for tags 5 and 6, targeting closed
sectors 77 and 145. Trigger lines activate only after accepted player movement;
the one-shot special clears once crossed. This follows id Software's
[`P_CrossSpecialLine` open-door action](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_spec.c).
Crossing a special-4 line raises its tagged door, waits 150 tics, then closes
it and consumes the line. Players or living enemies can activate it, matching
id Software's [`P_CrossSpecialLine`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_spec.c)
and [`EV_DoDoor`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_doors.c).
Crossing a special-10 line triggers its tagged platform down, wait, and up once,
then clears the line. Players and living enemies can activate it, matching id
Software's [`P_CrossSpecialLine` platform action](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_spec.c).
Player or living-enemy crossings of repeatable special-88 lines trigger the tagged platform to
descend at 140 units per second to the lowest neighboring floor, wait 105 tics
(3 seconds), then return to its starting height. The release WAD has five such
lines for tags 1 and 2, targeting sectors 98 and 103. Platform floors and their
collision and rendered geometry move together. The speed and timing follow
id Software's [`T_PlatRaise` and `downWaitUpStay` action](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_plats.c).
Press `E` from the front of a special-62 line to activate the same repeatable
down-wait-up action. E1M1 has four such lines for tags 1 and 2, targeting the
same sectors 98 and 103. The original engine routes special 62 to the same
`downWaitUpStay` platform action as special 88, but through the use control
instead of a walk crossing, as shown in id Software's
[`P_UseSpecialLine`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_switch.c).
Press `E` at the front of a special-18 line to raise its tagged floors to the
next higher neighboring floor at 35 units per second, then leave them there.
The one-shot line clears only when a floor starts moving. The target and speed
follow id Software's [`P_UseSpecialLine`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_switch.c)
and [`EV_DoFloor`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_floor.c).
Press `E` at the front of the one-shot special-23 line to lower its tagged
sectors to their lowest neighboring floor and leave them there. The WAD's one
tag-3 line targets sectors 76, 126, and 129; their floor heights lower from
272, 264, and 264 to 136, 144, and 136 at 35 units per second. It follows
id Software's
[`P_UseSpecialLine` and `lowerFloorToLowest` action](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_switch.c)
and [`EV_DoFloor`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_floor.c).
Crossing a special-38 line applies the same lower-to-lowest floor motion once,
then clears the line. It is player-triggered, following id Software's
[`P_CrossSpecialLine`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_spec.c).
Collect a matching keycard or skull to open special-26 blue, special-27 yellow,
or special-28 red doors. The prototype recognizes all six card/skull map-thing
types and renders their WAD sprites; cards and skulls grant the same color key.
E1M1 has one blue-card thing (type 5), drawn with `BKEYA0`; the original engine
checks for the blue card or skull before opening these doors in
[`EV_VerticalDoor`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_doors.c).
Use-only special 31 opens a manual door and leaves it open. Specials 32, 33,
and 34 do the same for blue, red, and yellow locks; they require the matching
card or skull and clear the one-shot line after activation. The open-stay and
key mapping follow id Software's [`EV_VerticalDoor`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_doors.c)
and [`P_UseSpecialLine`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_switch.c).
Press `E` at a front-facing special-29 line to raise each tagged door, wait,
then close it. The one-shot trigger clears after a door starts moving, matching
the original `P_UseSpecialLine` action in
[`p_switch.c`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_switch.c).
Special 103 instead opens each tagged sector and leaves the doors open; its
one-shot line clears only after a tagged door starts, as in the original
[`P_UseSpecialLine`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_switch.c)
and [`EV_DoDoor`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_doors.c).
Specials 133, 135, and 137 open every tagged door at four times normal speed
and leave it open; they require the matching blue, red, or yellow card/skull.
The one-shot line clears only after a door starts, following the original
[`EV_DoLockedDoor`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_doors.c)
and [`P_UseSpecialLine`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_switch.c).
Other locked-door action variants, enemy-triggered platform actions besides
specials 10 and 88, and other line specials remain unsupported.

E1M1 contains four secret sectors (sector special 9). Entering one increments
the secret counter in the window title once and clears its secret flag, matching
the original engine's
[`P_PlayerInSpecialSector`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_spec.c).
The WAD also has three sector-special-7 nukage sectors. They deal 5 HP every
32 game tics spent on the floor, following the original `P_PlayerInSpecialSector`
damage rule without suit mitigation.
Three sector-special-1 lights blink with randomized bright and dark intervals;
six sector-special-12 lights strobe together, alternating one bright tic, 35
dark tics, and five bright tics. Both use adjacent-sector minimum light levels
and rebuild the scene through SILICON when their levels change, following
id Software's [`T_LightFlash`, `T_StrobeFlash`, and spawn actions](https://raw.githubusercontent.com/id-Software/DOOM/master/linuxdoom-1.10/p_lights.c).
Blink timing uses the sample's seeded RNG rather than Doom's shared random table.

The combat slice loads four normal-skill enemy types from WAD things and their
classic `A1`–`D1` walk and `E1`–`G1` attack sprite patches: former humans (20
health), shotgunners (30), imps (60), and demons (150), including their
species-specific death patches. Moving enemies cycle their walk frames, play
three-frame attack poses and normal death sequences, and choose among eight
camera-relative views using Doom's state durations. Imps, former humans, and
shotgunners use their species' XDeath sequence when a hit leaves health below
negative spawn health; demons retain the normal death sequence because Doom
defines no SARG XDeath state. The extra frames and overkill threshold follow
id Software's [`P_KillMobj`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_inter.c)
and [monster state table](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/info.c).
The final death or gib frame remains as a corpse. Paired Freedoom patches are
horizontally flipped where Doom does so. Cutout billboards use a SILICON
fragment shader. With the
pistol selected, Space fires a seeded 5, 10, or 15-damage hitscan; holding it
repeats after 19 Doom tics (about 0.54 seconds), and pistol ammo caps at 200.
Damage follows id Software's [`P_GunShot`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_pspr.c);
the refire interval follows its pistol states and [`A_ReFire`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_pspr.c).
Press `1` for the pistol or `2` for the fist. A berserk pack selects the fist,
but you can switch back to the pistol. Space falls back to the fist when pistol ammo
runs out, matching Doom's [`P_CheckAmmo`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_pspr.c). With the fist selected, Space punches.
Press or hold `Q` for a fist punch with a 22-tic (about 0.63-second) cooldown,
no ammo cost, and Doom's randomized 2–20 damage; holding repeats punches when
ready. This held-attack behavior follows Doom's [`A_ReFire`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_pspr.c)
and pistol/fist state sequences in [`info.c`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/info.c).
It hits the nearest living enemy whose
16-unit radius intersects the forward trace within 64 units, unless a blocking
line comes first. The hit is immediate; PUNGC0, PUNGD0, PUNGC0, and PUNGB0
play afterward for 4, 5, 4, and 5 Doom tics. The startup frame is omitted; the
remaining pose durations follow id Software's
[weapon states](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/info.c).
The punch range and damage follow id Software's [`A_Punch`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_pspr.c)
and [`MELEERANGE`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_local.h).
A PSTR berserk pack restores up to 100 health without reducing health above 100,
then selects the fist for Space and multiplies fist damage by ten until the map
ends. Its healing, fist selection, and power follow id Software's
[`P_TouchSpecialThing` and `P_GivePower`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_inter.c);
the multiplier follows [`A_Punch`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_pspr.c).
Every fired shot alerts living enemies through open two-sided sectors, even
when it misses, crossing at most one sound-blocking linedef. This models Doom's
pistol noise alert and recursive sector sound flood, not every sound event.
Awake enemies deal 8 melee damage within 48 units, at most once every 0.85
seconds. Former humans fire 3-damage hitscan attacks and
shotgunners fire 6-damage hitscan attacks within 512 units, at most once every
1.4 seconds and only with clear sight past blocking lines. Damage and projectile
launch happen as soon as the cooldown expires; the pose plays afterward, with
no attack windup or aim spread. A surviving pistol hit rolls against Doom's
pain chances: 200/256 for imps and former humans, 180/256 for demons, and
170/256 for shotgunners. Successful rolls show the pain sprite for four tics
on imps and demons or six tics on former humans and shotgunners. A deterministic
local xorshift supplies rolls, so probabilities match but Doom's global random
sequence does not. The window title reports health, armor points and class,
ammo, pickups, kills, draw calls, and triangles. Melee, ranged, fireball, and
nukage damage all use the same armor calculation. Imps launch a
3D straight BAL1A0 fireball aimed at the player's body midpoint within 512
units when they have clear sight. It travels at 180 units per second, lasts up
to 3 seconds, and deals 8 damage on contact when the projectile height overlaps
the player, with a 2-second launch cooldown. Its z position follows a straight
line between the shooter and player's floor-based body midpoints. On wall or
player impact it plays BAL1C0–BAL1E0 for six Doom tics each. The WAD
pistol's PISGA0 patch is the idle camera-aligned billboard. On Space, the hit
happens immediately, then PISGC0 shows recoil for four Doom tics and PISGB0
shows recovery for five; those pose lengths follow id Software's
[pistol states](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/info.c).
Both weapon poses use SILICON's cutout shader and draw pipeline. The four-tic
pre-shot windup and remaining idle tics are not modeled. Enemies cycle their A/B ten-tic stand states while unaware.
They wake within 640 map units on clear sight or when hit; sight refreshes a
100-tic target timeout, during which they pursue through lost sight. All enemy
attacks require clear sight. When both positions resolve to BSP sectors, a
breadth-first route over walkable two-sided linedefs selects the next portal;
the opening must fit the actor and allow an upward step of at most 24 units. The
router samples portal points and picks one with a clear 16-unit swept margin
from blocking lines, allowing simple pursuit around walls. If map data yields
no waypoint, direct pursuit remains the fallback. This sector graph is not
Doom's full actor navigation; full target-selection rules and other sound
events remain unimplemented. See id Software's [`P_NoiseAlert` and
`P_RecursiveSound`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_enemy.c)
and [`P_FireWeapon`](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_pspr.c)
for the original sector sound flood and pistol alert call.

Freedoom 0.13.0 E1M1 has 29 normal-skill enemy placements. The sample parses
the WAD node partition tree to locate each thing's subsector and sector and to
cull map subsectors whose node bounds lie wholly outside the horizontal view
cone and near/far interval. Traversal visits child nodes near-to-far; leaves whose
mesh bounds exceed the intersected WAD bounds are tested separately. The player's
leaf is always retained. Each static
material mesh also uses its vertex bounds to reject geometry outside any
of the six camera-frustum planes, then surviving geometry is batched by
texture and cutout mode within coarse 2,048-unit view-depth bands, so nearby
bands reach the depth test first. This can reject hidden fragments before the
fragment shader runs, while preserving the framebuffer. Opaque, unmasked wall
quads add vertical spans to each fully covered screen column. For a later mesh,
the culler unions nearer spans within each column and rejects it only when their
combined coverage contains its full projected bounds in every covered column.
Bounds crossing uncovered wall edges or portal openings stay visible; this is
not Doom's exact per-column portal clipping. At the
checked-in 960×720 camera pose, 570 of 682 subsectors remain in the horizontal
BSP view. The
pre-occlusion checked-in capture measured 5,713 triangles across 260 draws.
The screenshots, draw counts, and timing measurements below are historical
captures from before this culler; current WAD render counts have not been
remeasured. The checked-in camera view
contains 59 visible pickup billboards: nine health/ammo items, 30 health
bonuses, one blue card, one green armor, and 18 armor bonuses; the armor items
do not appear in that capture.
The sky draw follows opaque map batches with `LessEqual` depth testing and depth
writes disabled, so the rasterizer rejects sky samples behind nearer map
geometry before it runs the sky shader. The panorama sits 64 map units inside
the 8,192-unit far clip plane to leave room for f32 transform rounding. Five
pre-occlusion release renders on Apple M2 measured a median process CPU time of
0.60 s;
the E1M1 output is byte-identical to the preceding capture. The earlier
depth-band comparison measured 0.62 s against 0.69 s at its predecessor. These
fixed-pose measurements are not engine-wide benchmarks. Earlier opaque-only counts were
3,896 triangles and 164 draws for this view. The screenshot shows the player
start, pistol, and a medikit; enemies and the newly supported armor items are
outside that camera view. A temporary
WAD with only its player start moved was used to capture an enemy sprite in view;
that test fixture is not included.

This is a limited gameplay prototype, not Doom's complete player physics or
game rules. Frustum bounds reject only map geometry outside the view; Doom's
detailed actor navigation, other sound events, other
power-up effects beyond health bonuses, soul spheres, radiation suits,
invulnerability spheres, partial-invisibility spheres, and light-amplification
visors, locked-door action variants, crossing specials other than 2 and 88,
other use specials, episode finales, weapons beyond the pistol and fist and
their ammunition, and complete weapon state sequences beyond the implemented
pistol and fist poses remain unimplemented.
`F_SKY1` ceilings use the map's episode sky texture, sampled by view angle. The
checked-in [`E1M1 screenshot`](../assets/screenshots/freedoom_e1m1.png) was
rendered from the unmodified release WAD. The WAD itself is not included. The release archive
checksum is SHA-256 `3f9b264f3e3ce503b4fb7f6bdcb1f419d93c7b546f4df3e874dd878db9688f59`.

![Freedoom former-human enemy and pistol billboards rendered through SILICON](../assets/screenshots/freedoom_e1m1_enemy.png)

![Freedoom E1M2 start view rendered through SILICON with the episode sky texture](../assets/screenshots/freedoom_e1m2.png)

*E1M2 capture after splitting BSP cells at sector boundaries and adding the WAD sky texture.*

![Freedoom E1M4 start view rendered through SILICON with a radiation-suit pickup in view](../assets/screenshots/freedoom_e1m4.png)

*E1M4 start view: 8,912 triangles across 379 draws in 829 of 1,202 horizontal leaves.*

*Sprite verification view from the same WAD with only the player start moved into the enemy corridor; the temporary WAD is not included.*

Freedoom's three-clause BSD notice and contributor list accompany this derived
sample in [`assets/licenses/FREEDOOM-COPYING.txt`](../assets/licenses/FREEDOOM-COPYING.txt)
and [`assets/licenses/FREEDOOM-CREDITS.txt`](../assets/licenses/FREEDOOM-CREDITS.txt).
The upstream project and contributors do not endorse SILICON. See the
[Freedoom 0.13.0 release](https://github.com/freedoom/freedoom/releases/tag/v0.13.0),
[license source](https://raw.githubusercontent.com/freedoom/freedoom/v0.13.0/COPYING.adoc),
[id Software's item, ammo, damage and target-threshold handling](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_inter.c),
[64-unit linedef use tracing](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_map.c),
[manual door height, speed, and wait handling](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_doors.c),
[special 11's level-exit action](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_switch.c),
[100-tic target threshold](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/p_local.h),
and id Software's [WAD](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/w_wad.h),
[map and BSP record](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/doomdata.h),
[BSP point traversal](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/r_main.c),
and [wall rendering](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/r_segs.c)
references. The walk frame sequence and durations follow id Software's
[monster states](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/info.c),
[sprite view selection](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/r_things.c),
and [35-tic game clock](https://github.com/id-Software/DOOM/blob/master/linuxdoom-1.10/doomdef.h).

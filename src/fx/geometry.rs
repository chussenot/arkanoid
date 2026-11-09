//! The effects' geometry: the shapes a quad cannot be, built with Lyon,
//! and the bounded per-frame triangle list both renderers draw.
//!
//! Three kinds of shape, three costs:
//! - **Templates**, tessellated once at start in unit space: the ring, the
//!   half-ring arc, the disc, a small library of shard and rock outlines.
//!   A stroked template keeps, per vertex, its point on the path and the
//!   stroke's offset at half-width 1, so an effect can draw it at any
//!   radius and any width without tessellating again.
//! - **Cracks and fracture pieces**, tessellated once per impact: the crack
//!   runs from the real contact point, and the pieces are cut along the
//!   same crack paths, so they fit together. They live in a fixed pool of
//!   meshes whose buffers are reused, never grown past their first use.
//! - **Quads** (glints, sparks, streaks, hairlines) go straight into the
//!   frame as four vertices: Lyon would add nothing to a rectangle.
//!
//! Everything is in field pixels. `FxVertex::height` lifts a shape off the
//! board in the 3D renderer; the 2D one ignores it.

use std::ops::Range;

use lyon_tessellation::math::point;
use lyon_tessellation::path::{Path, Winding};
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, LineCap, LineJoin, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};

/// One vertex of effect geometry, as the GPU reads it: field position,
/// height above the board (3D only), premultiplied linear color whose rgb
/// may exceed 1.0 (light, which the bloom picks up).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FxVertex {
    pub position: [f32; 2],
    pub height: f32,
    pub color: [f32; 4],
}

/// The frame's hard limits. Indices are 16-bit, so vertices stay under
/// 65,536; at these sizes a full frame is 690 KB of vertex data.
pub const MAX_VERTICES: usize = 24_576;
pub const MAX_INDICES: usize = 49_152;

/// A template or pooled mesh vertex: `p`, the point on the shape (or on
/// its path, for a stroke); `o`, the stroke's offset from it at half-width
/// 1 (zero for a fill); `t`, distance along the path in the path's units.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TVert {
    pub p: [f32; 2],
    pub o: [f32; 2],
    pub t: f32,
}

/// Part of a mesh drawn with one transform: a fracture piece, or a whole
/// template.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub vertices: Range<usize>,
    pub indices: Range<usize>,
    /// The part's center in the space its vertices were built in.
    pub center: [f32; 2],
}

/// Triangles with parts. Buffers keep their capacity when cleared, so a
/// pooled mesh stops allocating once it has held its largest shape.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<TVert>,
    pub indices: Vec<u16>,
    pub parts: Vec<Part>,
}

impl Mesh {
    fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.parts.clear();
    }

    /// Appends `buffers` as one part centered on `center`, with vertex
    /// positions made relative to it.
    fn push_part(&mut self, buffers: &VertexBuffers<TVert, u16>, center: [f32; 2]) {
        let v0 = self.vertices.len();
        let i0 = self.indices.len();
        self.vertices.extend(buffers.vertices.iter().map(|v| TVert {
            p: [v.p[0] - center[0], v.p[1] - center[1]],
            ..*v
        }));
        self.indices
            .extend(buffers.indices.iter().map(|&i| i + v0 as u16));
        self.parts.push(Part {
            vertices: v0..self.vertices.len(),
            indices: i0..self.indices.len(),
            center,
        });
    }
}

/// The effects' Lyon tessellators and scratch buffers, kept for the whole
/// session: tessellating reuses them instead of allocating.
pub struct Tessellator {
    fill: FillTessellator,
    stroke: StrokeTessellator,
    scratch: VertexBuffers<TVert, u16>,
}

impl Default for Tessellator {
    fn default() -> Self {
        Self {
            fill: FillTessellator::new(),
            stroke: StrokeTessellator::new(),
            scratch: VertexBuffers::with_capacity(512, 1024),
        }
    }
}

/// Curve flattening tolerance, in the units of the shape being built.
const UNIT_TOLERANCE: f32 = 0.004;
const PIXEL_TOLERANCE: f32 = 0.1;

fn fill_vertex(v: FillVertex) -> TVert {
    let p = v.position();
    TVert {
        p: [p.x, p.y],
        o: [0.0, 0.0],
        t: 0.0,
    }
}

/// A stroke vertex at half-width 1: its point on the path, and the offset
/// from there to the vertex.
fn stroke_vertex(v: StrokeVertex) -> TVert {
    let on = v.position_on_path();
    let at = v.position();
    TVert {
        p: [on.x, on.y],
        o: [at.x - on.x, at.y - on.y],
        t: v.advancement(),
    }
}

impl Tessellator {
    fn stroke(&mut self, path: &Path, options: &StrokeOptions) -> &VertexBuffers<TVert, u16> {
        self.scratch.vertices.clear();
        self.scratch.indices.clear();
        // A failed tessellation leaves a partial mesh; effects are never
        // worth a panic, so a broken shape just draws less.
        let _ = self.stroke.tessellate_path(
            path,
            options,
            &mut BuffersBuilder::new(&mut self.scratch, stroke_vertex),
        );
        &self.scratch
    }

    fn fill(&mut self, path: &Path, options: &FillOptions) -> &VertexBuffers<TVert, u16> {
        self.scratch.vertices.clear();
        self.scratch.indices.clear();
        let _ = self.fill.tessellate_path(
            path,
            options,
            &mut BuffersBuilder::new(&mut self.scratch, fill_vertex),
        );
        &self.scratch
    }
}

// -- templates ---------------------------------------------------------------

/// Distinct shard and rock outlines in the library.
pub const SHARDS: usize = 6;
pub const ROCKS: usize = 4;

/// The shapes every effect draws from, tessellated once.
pub struct Templates {
    /// A circle of radius 1, stroked: drawn at any radius and width.
    pub ring: Mesh,
    /// Half of that circle, centered on +x, with round caps: a rebound
    /// arc, turned to face the way the ball left.
    pub arc: Mesh,
    /// A filled circle of radius 1.
    pub disc: Mesh,
    /// Splinters of brick: angular, roughly unit size, centered.
    pub shards: [Mesh; SHARDS],
    /// Chips of asteroid: lumpy and notched, unlike a brick's clean break.
    pub rocks: [Mesh; ROCKS],
}

/// A deterministic stream of numbers in 0..1 from a seed (xorshift32):
/// an effect's variety comes from its event, never from a global RNG.
#[derive(Debug, Clone)]
pub struct Dice(u32);

impl Dice {
    pub fn new(seed: u32) -> Self {
        // Never zero: xorshift would stay there.
        Self(seed.wrapping_mul(0x9E37_79B9) | 1)
    }

    pub fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in `lo..hi`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

/// A closed polygon path through `points`.
fn polygon(points: &[[f32; 2]]) -> Path {
    let mut builder = Path::builder();
    builder.begin(point(points[0][0], points[0][1]));
    for p in &points[1..] {
        builder.line_to(point(p[0], p[1]));
    }
    builder.end(true);
    builder.build()
}

/// An outline of `corners` points around the origin, each at a radius in
/// `radius` and an angle jittered by up to `jitter` of its share of the
/// turn, then recentered on its own centroid.
fn outline(dice: &mut Dice, corners: usize, radius: (f32, f32), jitter: f32) -> Vec<[f32; 2]> {
    let step = std::f32::consts::TAU / corners as f32;
    let mut points: Vec<[f32; 2]> = (0..corners)
        .map(|i| {
            let angle = (i as f32 + dice.range(-jitter, jitter)) * step;
            let r = dice.range(radius.0, radius.1);
            [r * angle.cos(), r * angle.sin()]
        })
        .collect();
    let n = points.len() as f32;
    let c = points
        .iter()
        .fold([0.0, 0.0], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n]);
    for p in &mut points {
        *p = [p[0] - c[0], p[1] - c[1]];
    }
    points
}

impl Templates {
    pub fn build(tess: &mut Tessellator) -> Self {
        let unit_stroke = StrokeOptions::tolerance(UNIT_TOLERANCE)
            .with_line_width(2.0)
            .with_line_join(LineJoin::Round);
        let unit_fill = FillOptions::tolerance(UNIT_TOLERANCE);

        let mut ring = Mesh::default();
        let mut builder = Path::builder();
        builder.add_circle(point(0.0, 0.0), 1.0, Winding::Positive);
        ring.push_part(tess.stroke(&builder.build(), &unit_stroke), [0.0, 0.0]);

        let mut arc = Mesh::default();
        let mut builder = Path::svg_builder();
        builder.move_to(point(0.0, -1.0));
        builder.arc(
            point(0.0, 0.0),
            lyon_tessellation::math::vector(1.0, 1.0),
            lyon_tessellation::math::Angle::radians(std::f32::consts::PI),
            lyon_tessellation::math::Angle::radians(0.0),
        );
        let options = unit_stroke.with_line_cap(LineCap::Round);
        arc.push_part(tess.stroke(&builder.build(), &options), [0.0, 0.0]);

        let mut disc = Mesh::default();
        let mut builder = Path::builder();
        builder.add_circle(point(0.0, 0.0), 1.0, Winding::Positive);
        disc.push_part(tess.fill(&builder.build(), &unit_fill), [0.0, 0.0]);

        let mut dice = Dice::new(0x5EED_B1C5);
        let shards = std::array::from_fn(|_| {
            let corners = 3 + (dice.next() * 3.0) as usize;
            let points = outline(&mut dice, corners, (0.55, 1.0), 0.3);
            let mut mesh = Mesh::default();
            mesh.push_part(tess.fill(&polygon(&points), &unit_fill), [0.0, 0.0]);
            mesh
        });
        let rocks = std::array::from_fn(|_| {
            let corners = 7 + (dice.next() * 3.0) as usize;
            let points = outline(&mut dice, corners, (0.45, 1.0), 0.35);
            let mut mesh = Mesh::default();
            mesh.push_part(tess.fill(&polygon(&points), &unit_fill), [0.0, 0.0]);
            mesh
        });
        Self {
            ring,
            arc,
            disc,
            shards,
            rocks,
        }
    }
}

// -- cracks and fracture -----------------------------------------------------

/// Points along one crack arm, origin first, the brick's edge last.
pub const ARM_POINTS: usize = 6;
pub const MAX_ARMS: usize = 4;

/// A crack in a brick: arms from the impact point to the brick's edges.
/// Fixed-size, so a level's worth of them never allocates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crack {
    /// The brick's rect: center x, y, width, height.
    pub rect: [f32; 4],
    pub origin: [f32; 2],
    pub arms: [[[f32; 2]; ARM_POINTS]; MAX_ARMS],
    pub arm_count: usize,
}

/// Where a ray from `origin` (inside the rect) along `dir` leaves it.
fn exit_point(rect: [f32; 4], origin: [f32; 2], dir: [f32; 2]) -> [f32; 2] {
    let (hw, hh) = (rect[2] / 2.0, rect[3] / 2.0);
    let (lo, hi) = ([rect[0] - hw, rect[1] - hh], [rect[0] + hw, rect[1] + hh]);
    let mut t = f32::INFINITY;
    for axis in 0..2 {
        if dir[axis].abs() > 1e-6 {
            let bound = if dir[axis] > 0.0 { hi[axis] } else { lo[axis] };
            t = t.min((bound - origin[axis]) / dir[axis]);
        }
    }
    let t = if t.is_finite() { t.max(0.0) } else { 0.0 };
    [
        (origin[0] + dir[0] * t).clamp(lo[0], hi[0]),
        (origin[1] + dir[1] * t).clamp(lo[1], hi[1]),
    ]
}

impl Crack {
    /// A crack from where `at` (the striker's center) met the brick,
    /// running into it away from `normal` (the face's outward normal):
    /// three or four arms fanning out to the edges, each a jagged line.
    pub fn new(rect: [f32; 4], at: [f32; 2], normal: [f32; 2], seed: u32) -> Self {
        let mut dice = Dice::new(seed);
        let (hw, hh) = (rect[2] / 2.0, rect[3] / 2.0);
        // The impact point: the striker's center pulled onto the brick, a
        // pixel inside it.
        let origin = [
            (at[0] - normal[0]).clamp(rect[0] - hw + 1.0, rect[0] + hw - 1.0),
            (at[1] - normal[1]).clamp(rect[1] - hh + 1.0, rect[1] + hh - 1.0),
        ];
        let base = (-normal[1]).atan2(-normal[0]);
        let arm_count = 3 + usize::from(dice.next() > 0.5);
        let spread: &[f32] = if arm_count == 3 {
            &[-1.25, 0.0, 1.25]
        } else {
            &[-1.4, -0.45, 0.45, 1.4]
        };
        let mut arms = [[[0.0; 2]; ARM_POINTS]; MAX_ARMS];
        for (arm, &offset) in arms.iter_mut().zip(spread) {
            let angle = base + offset + dice.range(-0.2, 0.2);
            let dir = [angle.cos(), angle.sin()];
            let end = exit_point(rect, origin, dir);
            let perp = [-dir[1], dir[0]];
            for (i, point) in arm.iter_mut().enumerate() {
                let f = i as f32 / (ARM_POINTS - 1) as f32;
                // Jagged in the middle, pinned at both ends.
                let jag = if i == 0 || i == ARM_POINTS - 1 {
                    0.0
                } else {
                    dice.range(-2.4, 2.4)
                };
                *point = [
                    origin[0] + (end[0] - origin[0]) * f + perp[0] * jag,
                    origin[1] + (end[1] - origin[1]) * f + perp[1] * jag,
                ];
            }
        }
        Self {
            rect,
            origin,
            arms,
            arm_count,
        }
    }

    pub fn arms(&self) -> &[[[f32; 2]; ARM_POINTS]] {
        &self.arms[..self.arm_count]
    }

    /// The crack stroked as one mesh: `t` is each vertex's distance from
    /// the impact point along its arm, so an effect can grow it outward.
    pub fn stroke_into(&self, tess: &mut Tessellator, width: f32, out: &mut Mesh) {
        out.clear();
        let mut builder = Path::builder();
        for arm in self.arms() {
            builder.begin(point(arm[0][0], arm[0][1]));
            for p in &arm[1..] {
                builder.line_to(point(p[0], p[1]));
            }
            builder.end(false);
        }
        let options = StrokeOptions::tolerance(PIXEL_TOLERANCE)
            .with_line_width(width)
            .with_line_join(LineJoin::Round)
            .with_line_cap(LineCap::Round);
        // Stroked at its final width: `p + o` is the vertex, and `o` is
        // already scaled.
        out.push_part(tess.stroke(&builder.build(), &options), [0.0, 0.0]);
    }

    /// The brick cut along the crack: one piece between each pair of
    /// neighboring arms, bounded by the two arms and the stretch of the
    /// brick's edge between their ends. The pieces share their edges with
    /// the crack, so they fit back together. Each part's center is its
    /// centroid, in field pixels.
    pub fn fracture_into(&self, tess: &mut Tessellator, out: &mut Mesh) {
        out.clear();
        let [cx, cy, w, h] = self.rect;
        let (left, top) = (cx - w / 2.0, cy - h / 2.0);
        let perimeter = 2.0 * (w + h);
        // Distance clockwise around the edge from the top-left corner.
        let along = |p: [f32; 2]| -> f32 {
            let (dx, dy) = (p[0] - left, p[1] - top);
            if dy <= 0.01 {
                dx
            } else if dx >= w - 0.01 {
                w + dy
            } else if dy >= h - 0.01 {
                w + h + (w - dx)
            } else {
                2.0 * w + h + (h - dy)
            }
        };
        let corners = [
            (0.0, [left, top]),
            (w, [left + w, top]),
            (w + h, [left + w, top + h]),
            (2.0 * w + h, [left, top + h]),
        ];
        let mut order = [0usize; MAX_ARMS];
        for (i, o) in order.iter_mut().enumerate().take(self.arm_count) {
            *o = i;
        }
        let order = &mut order[..self.arm_count];
        order.sort_by(|&a, &b| {
            along(self.arms[a][ARM_POINTS - 1]).total_cmp(&along(self.arms[b][ARM_POINTS - 1]))
        });
        let fill = FillOptions::tolerance(PIXEL_TOLERANCE);
        let mut outline: [[f32; 2]; 2 * ARM_POINTS + 4] = [[0.0; 2]; 2 * ARM_POINTS + 4];
        for k in 0..order.len() {
            let a = &self.arms[order[k]];
            let b = &self.arms[order[(k + 1) % order.len()]];
            let (sa, mut sb) = (along(a[ARM_POINTS - 1]), along(b[ARM_POINTS - 1]));
            if sb <= sa {
                sb += perimeter;
            }
            let mut n = 0;
            for p in a {
                outline[n] = *p;
                n += 1;
            }
            for lap in [0.0, perimeter] {
                for &(s, corner) in &corners {
                    if s + lap > sa && s + lap < sb {
                        outline[n] = corner;
                        n += 1;
                    }
                }
            }
            for p in b[1..].iter().rev() {
                outline[n] = *p;
                n += 1;
            }
            let shape = &outline[..n];
            let inv = 1.0 / n as f32;
            let center = shape
                .iter()
                .fold([0.0, 0.0], |c, p| [c[0] + p[0] * inv, c[1] + p[1] * inv]);
            out.push_part(tess.fill(&polygon(shape), &fill), center);
        }
    }
}

// -- the frame's triangles ---------------------------------------------------

/// One frame of effect geometry: a bounded triangle list. Allocated once
/// at its limits; filling it never allocates, and a shape that would
/// overflow it is dropped whole.
#[derive(Debug)]
pub struct Canvas {
    pub vertices: Vec<FxVertex>,
    pub indices: Vec<u16>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            vertices: Vec::with_capacity(MAX_VERTICES),
            indices: Vec::with_capacity(MAX_INDICES),
        }
    }
}

impl Canvas {
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
    }

    fn room(&self, vertices: usize, indices: usize) -> bool {
        self.vertices.len() + vertices <= MAX_VERTICES
            && self.indices.len() + indices <= MAX_INDICES
    }

    /// A quad through four corners in order around it, each with its own
    /// color (a gradient when they differ).
    pub fn quad(&mut self, corners: [[f32; 2]; 4], height: f32, colors: [[f32; 4]; 4]) {
        if !self.room(4, 6) {
            return;
        }
        let base = self.vertices.len() as u16;
        for (position, color) in corners.into_iter().zip(colors) {
            self.vertices.push(FxVertex {
                position,
                height,
                color,
            });
        }
        self.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// A straight stroke from `a` to `b`, `w0` wide at `a` tapering to
    /// `w1` at `b`, its color running from `c0` to `c1`.
    #[allow(clippy::too_many_arguments)]
    pub fn segment(
        &mut self,
        a: [f32; 2],
        b: [f32; 2],
        w0: f32,
        w1: f32,
        height: f32,
        c0: [f32; 4],
        c1: [f32; 4],
    ) {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy);
        if len < 1e-4 {
            return;
        }
        let n = [-dy / len, dx / len];
        let (h0, h1) = (w0 / 2.0, w1 / 2.0);
        self.quad(
            [
                [a[0] + n[0] * h0, a[1] + n[1] * h0],
                [b[0] + n[0] * h1, b[1] + n[1] * h1],
                [b[0] - n[0] * h1, b[1] - n[1] * h1],
                [a[0] - n[0] * h0, a[1] - n[1] * h0],
            ],
            height,
            [c0, c1, c1, c0],
        );
    }

    /// One part of a mesh, each vertex placed by `place`, which returns
    /// its field position and color (or `None` to leave the part out).
    pub fn part(
        &mut self,
        mesh: &Mesh,
        part: usize,
        height: f32,
        mut place: impl FnMut(&TVert) -> ([f32; 2], [f32; 4]),
    ) {
        let Some(part) = mesh.parts.get(part) else {
            return;
        };
        if !self.room(part.vertices.len(), part.indices.len()) {
            return;
        }
        let base = self.vertices.len();
        for v in &mesh.vertices[part.vertices.clone()] {
            let (position, color) = place(v);
            self.vertices.push(FxVertex {
                position,
                height,
                color,
            });
        }
        let first = part.vertices.start;
        self.indices.extend(
            mesh.indices[part.indices.clone()]
                .iter()
                .map(|&i| (i as usize - first + base) as u16),
        );
    }

    /// A stroked template (ring, arc) at `center`, turned by `angle`, at
    /// `radius` with a stroke `width` wide, in one color.
    #[allow(clippy::too_many_arguments)]
    pub fn stroke(
        &mut self,
        mesh: &Mesh,
        center: [f32; 2],
        angle: f32,
        radius: f32,
        width: f32,
        height: f32,
        color: [f32; 4],
    ) {
        let (s, c) = angle.sin_cos();
        let half = width / 2.0;
        self.part(mesh, 0, height, |v| {
            let x = v.p[0] * radius + v.o[0] * half;
            let y = v.p[1] * radius + v.o[1] * half;
            (
                [center[0] + x * c - y * s, center[1] + x * s + y * c],
                color,
            )
        });
    }

    /// Light around a rect: `inner` over the rect itself, and a soft
    /// border `spread` wide outside it, from `edge` at the rect's edge to
    /// nothing. A glow with no hard outline, so a flash never reads as a
    /// box laid over the brick.
    pub fn glow_rect(
        &mut self,
        rect: [f32; 4],
        spread: f32,
        height: f32,
        inner: [f32; 4],
        edge: [f32; 4],
    ) {
        let [cx, cy, w, h] = rect;
        let (hw, hh) = (w / 2.0, h / 2.0);
        let i = [
            [cx - hw, cy - hh],
            [cx + hw, cy - hh],
            [cx + hw, cy + hh],
            [cx - hw, cy + hh],
        ];
        let (ow, oh) = (hw + spread, hh + spread);
        let o = [
            [cx - ow, cy - oh],
            [cx + ow, cy - oh],
            [cx + ow, cy + oh],
            [cx - ow, cy + oh],
        ];
        self.quad(i, height, [inner; 4]);
        let none = [0.0; 4];
        for k in 0..4 {
            let n = (k + 1) % 4;
            self.quad([i[k], o[k], o[n], i[n]], height, [edge, none, none, edge]);
        }
    }

    /// A filled template at `center`, turned by `angle`, scaled by
    /// `scale`, in one color.
    pub fn fill(
        &mut self,
        mesh: &Mesh,
        center: [f32; 2],
        angle: f32,
        scale: f32,
        height: f32,
        color: [f32; 4],
    ) {
        let (s, c) = angle.sin_cos();
        self.part(mesh, 0, height, |v| {
            let (x, y) = (v.p[0] * scale, v.p[1] * scale);
            (
                [center[0] + x * c - y * s, center[1] + x * s + y * c],
                color,
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_tessellate_to_closed_well_formed_meshes() {
        let mut tess = Tessellator::default();
        let t = Templates::build(&mut tess);
        let meshes = [&t.ring, &t.arc, &t.disc]
            .into_iter()
            .chain(&t.shards)
            .chain(&t.rocks);
        for mesh in meshes {
            assert!(!mesh.indices.is_empty());
            assert_eq!(mesh.indices.len() % 3, 0);
            assert!(mesh
                .indices
                .iter()
                .all(|&i| (i as usize) < mesh.vertices.len()));
        }
        // The ring's points sit on the unit circle, its offsets at
        // half-width 1: drawn at radius r and width w, it spans r ± w/2.
        for v in &t.ring.vertices {
            let r = v.p[0].hypot(v.p[1]);
            assert!((r - 1.0).abs() < 0.01, "{r}");
            assert!((v.o[0].hypot(v.o[1]) - 1.0).abs() < 0.01);
        }
    }

    #[test]
    fn a_crack_runs_from_the_impact_into_the_brick_to_its_edges() {
        let rect = [100.0, 50.0, 52.0, 22.0];
        // Struck on the bottom face, the ball below it.
        let crack = Crack::new(rect, [110.0, 67.0], [0.0, 1.0], 7);
        assert!(crack.arm_count >= 3);
        assert!((crack.origin[1] - 60.0).abs() < 1.5, "{:?}", crack.origin);
        for arm in crack.arms() {
            let end = arm[ARM_POINTS - 1];
            let on_edge = (end[0] - 74.0).abs() < 0.01
                || (end[0] - 126.0).abs() < 0.01
                || (end[1] - 39.0).abs() < 0.01
                || (end[1] - 61.0).abs() < 0.01;
            assert!(on_edge, "{end:?}");
            // Into the brick: up, away from the struck bottom face.
            assert!(end[1] < crack.origin[1] + 0.01);
        }
    }

    #[test]
    fn a_crack_is_the_same_for_the_same_impact() {
        let a = Crack::new([0.0, 0.0, 52.0, 22.0], [5.0, 17.0], [0.0, 1.0], 42);
        let b = Crack::new([0.0, 0.0, 52.0, 22.0], [5.0, 17.0], [0.0, 1.0], 42);
        assert_eq!(a, b);
    }

    #[test]
    fn the_fracture_pieces_tile_the_brick() {
        let mut tess = Tessellator::default();
        let rect = [100.0, 50.0, 52.0, 22.0];
        for seed in 0..16 {
            let crack = Crack::new(rect, [90.0 + seed as f32, 67.0], [0.0, 1.0], seed);
            let mut mesh = Mesh::default();
            crack.fracture_into(&mut tess, &mut mesh);
            assert_eq!(mesh.parts.len(), crack.arm_count);
            // Sum of triangle areas == the brick's, within the crack
            // lines' rounding: no gap, no overlap.
            let area: f32 = mesh
                .parts
                .iter()
                .map(|part| {
                    mesh.indices[part.indices.clone()]
                        .chunks(3)
                        .map(|t| {
                            let p = |i: u16| {
                                let v = mesh.vertices[i as usize].p;
                                [v[0] + part.center[0], v[1] + part.center[1]]
                            };
                            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
                            ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])).abs()
                                / 2.0
                        })
                        .sum::<f32>()
                })
                .sum();
            assert!(
                (area - 52.0 * 22.0).abs() < 52.0 * 22.0 * 0.03,
                "seed {seed}: {area}"
            );
        }
    }

    #[test]
    fn the_canvas_never_grows_past_its_limits() {
        let mut canvas = Canvas::default();
        let (vcap, icap) = (canvas.vertices.capacity(), canvas.indices.capacity());
        for _ in 0..(MAX_VERTICES / 4 + 100) {
            canvas.quad([[0.0; 2]; 4], 0.0, [[1.0; 4]; 4]);
        }
        assert_eq!(canvas.vertices.len(), MAX_VERTICES);
        assert_eq!(
            (canvas.vertices.capacity(), canvas.indices.capacity()),
            (vcap, icap),
            "no reallocation"
        );
    }
}

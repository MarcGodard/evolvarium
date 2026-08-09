// Atmospheric scattering ACROSS the planet disc, as seen from orbit.
//
// Distinct from viz::Atmosphere, which is front-culled and therefore only ever shows as a rim past the
// silhouette. This layer is the reason a water world looks blue from space at all: deep-ocean albedo is
// ~0.06, so the sea itself is nearly black and renders that way honestly. What the eye reads as "blue
// planet" is sunlight Rayleigh-scattered by the air column ABOVE the water, which is additive and lifts
// dark ocean far more than it lifts bright land.
//
// Two terms, both physical:
//   day  = how much sun reaches this bit of air at all (nothing on the night side).
//   path = air mass along the view ray, ~1/cos(view angle from local vertical), so the limb piles up much
//          more atmosphere than the sub-camera point and glows brighter. That gradient is what makes a
//          sphere read as a sphere instead of a flat blue disc.
use bevy::prelude::*;

#[derive(Component)]
pub struct AtmosphereHaze;

/// Shell radius as a multiple of PLANET_R. Above ELEV_MAX so mountains cannot poke through and z-fight,
/// below the rim shell at 1.17 so the two layers stack rather than intersect.
pub const HAZE_R: f32 = 1.10;

/// Peak scattered radiance at the sub-solar point looking straight down, in LINEAR light. Sized against the
/// surfaces it sits over: deep water is ~0.06 albedo and land ~0.2-0.3, so this roughly doubles the ocean
/// while leaving land a faint veil. Additive, so it is a LIFT on the surface, not a replacement.
const HAZE_GAIN: f32 = 0.075;

/// Air mass is 1/cos(theta) and diverges at the limb. Real atmospheres saturate instead (the ray exits the
/// top of the air column), so cap it; without the cap the limb blows to white.
const PATH_MAX: f32 = 2.2;

/// Rayleigh scattering goes as 1/lambda^4, so blue dominates hard. Not a pastel sky blue: this multiplies
/// into an additive layer, and a whitish value here greys the whole planet.
const RAYLEIGH: Vec3 = Vec3::new(0.22, 0.45, 1.0);

/// Warm band at the terminator: the grazing sun crosses a long air path, and the blue is scattered out of it
/// before it arrives. Same reason sunsets are red.
const TWILIGHT: Vec3 = Vec3::new(1.0, 0.42, 0.30);

pub fn spawn_haze(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) {
    let mut mesh = Sphere::new(crate::sphere::PLANET_R * HAZE_R).mesh().ico(5).unwrap();
    let n = mesh.count_vertices();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32, 0.0, 0.0, 1.0]; n]);
    commands.spawn((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::WHITE, // vertex colours carry all of it
            unlit: true,              // scattering is computed here, not by the scene light
            alpha_mode: AlphaMode::Add,
            ..default()
        })),
        Transform::IDENTITY,
        bevy::light::NotShadowCaster,
        Visibility::Hidden, // orbit only: from the ground this shell is overhead and would fog the sky
        AtmosphereHaze,
    ));
}

pub fn update_haze(
    mode: Res<crate::camera::CameraMode>,
    gen: Res<crate::sim::GenState>,
    offset: Res<crate::viz::SunOffset>,
    // OrbitCam, not any Camera3d: the minimap is a second Camera3d, so a bare single() finds two, returns
    // Err, and leaves the shell's vertex colours at their initial black. Additive black draws nothing, so the
    // whole layer silently vanishes rather than erroring.
    cam: Query<&GlobalTransform, With<crate::camera::OrbitCam>>,
    haze: Query<(&Mesh3d, &mut Visibility), With<AtmosphereHaze>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let orbit = *mode == crate::camera::CameraMode::Orbit;
    let mut haze = haze;
    let Ok((m3, mut vis)) = haze.single_mut() else { return };
    let want = if orbit { Visibility::Inherited } else { Visibility::Hidden };
    if *vis != want {
        *vis = want;
    }
    if !orbit {
        return;
    }
    let Ok(cam_tf) = cam.single() else { return };
    let eye = cam_tf.translation();
    let Some(mesh) = meshes.get_mut(&m3.0) else { return };
    let pos: Vec<[f32; 3]> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) => p.clone(),
        _ => return,
    };
    let vtick = (gen.tick as i64 + offset.0).max(0) as u32;
    let sun = crate::sphere::sun_dir(vtick);
    let cols: Vec<[f32; 4]> = pos
        .iter()
        .map(|p| {
            let world = Vec3::from_array(*p);
            let up = world.normalize_or_zero(); // local vertical (planet centred at origin)
            let lit = up.dot(sun); // -1 night .. 1 sub-solar
            // smoothstep across the terminator rather than a hard cut: air stays lit briefly past it.
            let t = ((lit + 0.20) / 0.45).clamp(0.0, 1.0);
            let day = t * t * (3.0 - 2.0 * t);
            let view = (eye - world).normalize_or_zero();
            let path = (1.0 / up.dot(view).max(0.05)).min(PATH_MAX);
            let warm = (1.0 - (lit.abs() / 0.20)).clamp(0.0, 1.0);
            let c = RAYLEIGH.lerp(TWILIGHT, warm * 0.55) * (HAZE_GAIN * day * path);
            [c.x, c.y, c.z, 1.0]
        })
        .collect();
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, cols);
}

#[cfg(test)]
mod tests {
    use super::*;

    // The limb must out-glow the sub-camera point or the planet reads as a flat disc. Guards the path term
    // against a future edit that drops the view dependence.
    #[test]
    fn limb_piles_up_more_air_than_straight_down() {
        let straight = 1.0f32 / 1.0f32.max(0.05);
        let grazing = (1.0f32 / 0.08f32).min(PATH_MAX);
        assert!(grazing > straight * 2.0, "limb should carry much more air mass");
        assert!(grazing <= PATH_MAX, "and must stay capped so it cannot blow to white");
    }

    // Night side gets no scattered sunlight. Without this the additive layer would light the dark side.
    #[test]
    fn night_side_scatters_nothing() {
        let lit = -0.8f32;
        let t = ((lit + 0.20) / 0.45).clamp(0.0, 1.0);
        assert_eq!(t, 0.0, "well past the terminator must be fully dark");
    }
}

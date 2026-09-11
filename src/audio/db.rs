/// Floor used for silence so the UI never has to display negative infinity.
pub const SILENCE_DB: f32 = -150.0;

pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        SILENCE_DB
    } else {
        20.0 * linear.log10()
    }
}

pub fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

pub fn power_to_db(power: f32) -> f32 {
    if power <= 0.0 {
        SILENCE_DB
    } else {
        10.0 * power.log10()
    }
}

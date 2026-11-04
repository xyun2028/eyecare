//! 色温到 Gamma 查找表的换算。
//!
//! 这一层是纯函数，不碰任何系统接口，方便单测和跨平台复用。
//!
//! Gamma 表的结构是 768 个 u16：前 256 个是红色通道，中间 256 个是绿色，
//! 最后 256 个是蓝色。每个位置代表「输入亮度 i」应该被映射成多少输出亮度。

pub mod display;

pub use display::{DisplayBackend, DisplayInfo};

/// 色温转 RGB 白点，各通道取值 0.0 ~ 1.0。
///
/// 用的是 Tanner Helland 的经典拟合公式，和 redshift / f.lux 一脉相承。
/// 6500K 附近基本是全白，往低色温走红色保留、蓝色衰减。
pub fn kelvin_to_rgb(kelvin: u32) -> (f64, f64, f64) {
    let t = kelvin.clamp(1000, 40000) as f64 / 100.0;

    let r = if t <= 66.0 {
        255.0
    } else {
        329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2)
    };

    let g = if t <= 66.0 {
        99.470_802_586_1 * t.ln() - 161.119_568_166_1
    } else {
        288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
    };

    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
    };

    let norm = |v: f64| (v / 255.0).clamp(0.0, 1.0);
    (norm(r), norm(g), norm(b))
}

/// 生成一张 Gamma 表。
///
/// - `kelvin`：目标色温，6500 是标准白光
/// - `brightness`：整体亮度系数，1.0 表示不动，只做色温偏移
///
/// 三个通道会先按最大通道归一化，保证最亮的那一路仍然是 1.0，
/// 否则调色温会连带把整块屏幕压暗，用户会以为是亮度变了。
pub fn build_gamma_ramp(kelvin: u32, brightness: f64) -> [u16; 768] {
    let (r, g, b) = kelvin_to_rgb(kelvin);
    let peak = r.max(g).max(b).max(f64::MIN_POSITIVE);

    let mut scale = [r / peak, g / peak, b / peak];
    for s in scale.iter_mut() {
        *s = (*s * brightness).clamp(0.0, 1.0);
    }

    let mut ramp = [0u16; 768];
    for i in 0..256usize {
        let input = i as f64 / 255.0;
        for (ch, s) in scale.iter().enumerate() {
            ramp[ch * 256 + i] = (input * s * 65535.0).round().clamp(0.0, 65535.0) as u16;
        }
    }
    ramp
}

/// 按用户自选的颜色生成 Gamma 表。
///
/// 和色温的区别：色温是「物理上更暖」，会保留峰值通道；
/// 这里是「盖一层滤色片」，每个通道按所选颜色的比例压低，
/// 所以画面整体会变暗一点，这是滤色本身的代价，不是 bug。
pub fn build_gamma_ramp_rgb(rgb: [u8; 3], brightness: f64) -> [u16; 768] {
    let mut ramp = [0u16; 768];
    for (ch, value) in rgb.iter().enumerate() {
        let scale = (*value as f64 / 255.0 * brightness).clamp(0.0, 1.0);
        for i in 0..256usize {
            let input = i as f64 / 255.0;
            ramp[ch * 256 + i] = (input * scale * 65535.0).round().clamp(0.0, 65535.0) as u16;
        }
    }
    ramp
}

/// 原始表：输入多少输出多少，用来恢复屏幕。
pub fn identity_ramp() -> [u16; 768] {
    let mut ramp = [0u16; 768];
    for ch in 0..3 {
        for i in 0..256usize {
            ramp[ch * 256 + i] = (i as f64 / 255.0 * 65535.0).round() as u16;
        }
    }
    ramp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 标准白光接近原样() {
        let ramp = build_gamma_ramp(6500, 1.0);
        for i in 0..256 {
            let identity = (i as f64 / 255.0 * 65535.0).round() as i32;
            for ch in 0..3 {
                let diff = (ramp[ch * 256 + i] as i32 - identity).abs();
                assert!(
                    diff < 1500,
                    "通道 {ch} 第 {i} 项偏差过大：{} vs {}",
                    ramp[ch * 256 + i],
                    identity
                );
            }
        }
    }

    #[test]
    fn 低色温蓝色被压低() {
        let ramp = build_gamma_ramp(3400, 1.0);
        let red_top = ramp[255] as u32;
        let blue_top = ramp[512 + 255] as u32;
        assert!(
            blue_top < red_top,
            "3400K 时蓝色峰值应低于红色：blue={blue_top} red={red_top}"
        );
        assert!(blue_top > 0, "蓝色不应该被压到全黑");
    }

    #[test]
    fn 各通道单调不递减() {
        let ramp = build_gamma_ramp(4000, 1.0);
        for ch in 0..3 {
            for i in 1..256 {
                assert!(
                    ramp[ch * 256 + i] >= ramp[ch * 256 + i - 1],
                    "通道 {ch} 在 {i} 处出现回落，曲线会断档"
                );
            }
        }
    }

    #[test]
    fn 亮度系数整体缩放() {
        let full = build_gamma_ramp(5000, 1.0);
        let half = build_gamma_ramp(5000, 0.5);
        assert!(half[255] < full[255]);
        assert!(half[255] > 0);
    }

    #[test]
    fn 原始表两端口值() {
        let ramp = identity_ramp();
        assert_eq!(ramp[0], 0);
        assert_eq!(ramp[255], 65535);
        assert_eq!(ramp[256], 0);
        assert_eq!(ramp[767], 65535);
    }
}

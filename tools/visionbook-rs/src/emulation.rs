// Mobile device emulation using Chrome DevTools Protocol

use crate::types::{MobileDevice, VisionResult};

pub struct DeviceEmulator {
    device: MobileDevice,
}

impl DeviceEmulator {
    pub fn new(device: MobileDevice) -> Self {
        Self { device }
    }

    /// Get device viewport dimensions and scale factor
    pub fn get_viewport(&self) -> (u32, u32, f64) {
        self.device.viewport()
    }

    /// Get device user agent string
    pub fn get_user_agent(&self) -> String {
        match &self.device {
            MobileDevice::IPhone12 | MobileDevice::IPhone13Pro | MobileDevice::IPhone14ProMax => {
                "Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Mobile/15E148 Safari/604.1".to_string()
            }
            MobileDevice::IPhoneSE => {
                "Mozilla/5.0 (iPhone; CPU iPhone OS 15_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.0 Mobile/15E148 Safari/604.1".to_string()
            }
            MobileDevice::GalaxyS21 | MobileDevice::GalaxyS22Ultra => {
                "Mozilla/5.0 (Linux; Android 12; SM-G991B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/108.0.0.0 Mobile Safari/537.36".to_string()
            }
            MobileDevice::Pixel5 | MobileDevice::Pixel7Pro => {
                "Mozilla/5.0 (Linux; Android 13; Pixel 7 Pro) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/108.0.0.0 Mobile Safari/537.36".to_string()
            }
            MobileDevice::IPadPro | MobileDevice::IPadMini => {
                "Mozilla/5.0 (iPad; CPU OS 16_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.0 Mobile/15E148 Safari/604.1".to_string()
            }
            MobileDevice::Custom { user_agent, .. } => user_agent.clone(),
        }
    }

    /// Get device name for logging
    pub fn get_device_name(&self) -> &str {
        match &self.device {
            MobileDevice::IPhone12 => "iPhone 12",
            MobileDevice::IPhone13Pro => "iPhone 13 Pro",
            MobileDevice::IPhone14ProMax => "iPhone 14 Pro Max",
            MobileDevice::IPhoneSE => "iPhone SE",
            MobileDevice::GalaxyS21 => "Galaxy S21",
            MobileDevice::GalaxyS22Ultra => "Galaxy S22 Ultra",
            MobileDevice::Pixel5 => "Pixel 5",
            MobileDevice::Pixel7Pro => "Pixel 7 Pro",
            MobileDevice::IPadPro => "iPad Pro",
            MobileDevice::IPadMini => "iPad Mini",
            MobileDevice::Custom { .. } => "Custom Device",
        }
    }

    /// Apply emulation settings to browser page
    /// This would integrate with chromiumoxide's Emulation domain
    pub async fn apply(&self) -> VisionResult<()> {
        let (width, height, scale) = self.get_viewport();
        let user_agent = self.get_user_agent();

        tracing::info!(
            "Emulating {} ({}x{} @ {}x, UA: {}...)",
            self.get_device_name(),
            width,
            height,
            scale,
            &user_agent[..50.min(user_agent.len())]
        );

        // Placeholder for CDP integration
        // In real implementation, would call:
        // - Emulation.setDeviceMetricsOverride(width, height, scale, mobile: true)
        // - Emulation.setUserAgentOverride(user_agent)
        // - Emulation.setTouchEmulationEnabled(true)

        Ok(())
    }
}

/// Parse device string to MobileDevice enum
pub fn parse_device(device_str: &str) -> Option<MobileDevice> {
    match device_str.to_lowercase().as_str() {
        "iphone12" | "iphone-12" => Some(MobileDevice::IPhone12),
        "iphone13pro" | "iphone-13-pro" => Some(MobileDevice::IPhone13Pro),
        "iphone14promax" | "iphone-14-pro-max" => Some(MobileDevice::IPhone14ProMax),
        "iphonese" | "iphone-se" => Some(MobileDevice::IPhoneSE),
        "galaxys21" | "galaxy-s21" => Some(MobileDevice::GalaxyS21),
        "galaxys22ultra" | "galaxy-s22-ultra" => Some(MobileDevice::GalaxyS22Ultra),
        "pixel5" | "pixel-5" => Some(MobileDevice::Pixel5),
        "pixel7pro" | "pixel-7-pro" => Some(MobileDevice::Pixel7Pro),
        "ipadpro" | "ipad-pro" => Some(MobileDevice::IPadPro),
        "ipadmini" | "ipad-mini" => Some(MobileDevice::IPadMini),
        _ => None,
    }
}

/// List all available device presets
pub fn list_devices() -> Vec<(&'static str, &'static str)> {
    vec![
        ("iphone12", "iPhone 12 - 390x844 @ 3x"),
        ("iphone13pro", "iPhone 13 Pro - 390x844 @ 3x"),
        ("iphone14promax", "iPhone 14 Pro Max - 430x932 @ 3x"),
        ("iphonese", "iPhone SE - 375x667 @ 2x"),
        ("galaxys21", "Galaxy S21 - 360x800 @ 3x"),
        ("galaxys22ultra", "Galaxy S22 Ultra - 412x915 @ 3.5x"),
        ("pixel5", "Pixel 5 - 393x851 @ 2.75x"),
        ("pixel7pro", "Pixel 7 Pro - 412x892 @ 3.5x"),
        ("ipadpro", "iPad Pro - 1024x1366 @ 2x"),
        ("ipadmini", "iPad Mini - 768x1024 @ 2x"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_device() {
        assert!(parse_device("iphone12").is_some());
        assert!(parse_device("pixel-7-pro").is_some());
        assert!(parse_device("invalid").is_none());
    }

    #[test]
    fn test_device_viewport() {
        let emulator = DeviceEmulator::new(MobileDevice::IPhone12);
        let (width, height, scale) = emulator.get_viewport();
        assert_eq!(width, 390);
        assert_eq!(height, 844);
        assert_eq!(scale, 3.0);
    }

    #[test]
    fn test_list_devices() {
        let devices = list_devices();
        assert!(!devices.is_empty());
        assert_eq!(devices.len(), 10);
    }
}

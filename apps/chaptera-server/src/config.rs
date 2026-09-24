use std::{
    env,
    error::Error,
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

pub const DEFAULT_LISTEN: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub listen: SocketAddr,
}

impl RuntimeConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let listen = match env::var("CHAPTERA_LISTEN") {
            Ok(raw) => raw.parse().map_err(|_| ConfigError::InvalidListen(raw))?,
            Err(env::VarError::NotPresent) => DEFAULT_LISTEN,
            Err(env::VarError::NotUnicode(_)) => {
                return Err(ConfigError::InvalidListen(
                    "CHAPTERA_LISTEN is not valid UTF-8".to_owned(),
                ));
            }
        };

        let config = Self { listen };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if is_private_listener(self.listen.ip()) {
            Ok(())
        } else {
            Err(ConfigError::PublicListenerForbidden(self.listen))
        }
    }
}

fn is_private_listener(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
        IpAddr::V6(ip) => ip.is_loopback() || is_unique_local(ip),
    }
}

fn is_unique_local(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xfe00 == 0xfc00
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    InvalidListen(String),
    PublicListenerForbidden(SocketAddr),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidListen(value) => {
                write!(formatter, "invalid CHAPTERA_LISTEN value: {value}")
            }
            Self::PublicListenerForbidden(address) => write!(
                formatter,
                "refusing public/unspecified application listener {address}; bind Chaptera to loopback or a private address behind the HTTPS edge"
            ),
        }
    }
}

impl Error for ConfigError {}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use super::RuntimeConfig;

    #[test]
    fn accepts_loopback_and_private_addresses() {
        for address in ["127.0.0.1:8080", "10.0.0.5:8080", "192.168.1.5:8080", "[fd00::5]:8080"] {
            let config = RuntimeConfig {
                listen: address.parse::<SocketAddr>().unwrap(),
            };
            assert!(config.validate().is_ok(), "{address}");
        }
    }

    #[test]
    fn rejects_public_and_unspecified_addresses() {
        for address in ["0.0.0.0:8080", "[::]:8080", "8.8.8.8:8080"] {
            let config = RuntimeConfig {
                listen: address.parse::<SocketAddr>().unwrap(),
            };
            assert!(config.validate().is_err(), "{address}");
        }
    }
}

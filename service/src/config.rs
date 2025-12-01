use std::{
    env,
    path::{
        Path,
        PathBuf,
    },
};

use figment::{
    Figment,
    providers::{
        self,
        Format,
    },
};
use serde::{
    Deserialize,
    Serialize,
};
use snafu::{
    ResultExt,
    Snafu,
    ensure,
};

use crate::{
    Mode,
    dirs::{
        Dirs,
        DirsError,
    },
};

const CONF_FILENAME: &str = "rinit.conf";

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    #[serde(flatten)]
    pub dirs: Dirs,
    pub mode: Mode,
}

#[derive(Debug, Snafu)]
pub enum ConfigError {
    #[snafu(display("unable to initialize XDG BaseDirectories"))]
    DirectoriesError { source: DirsError },
    #[snafu(display("unable to find configuration file {:?}", config_file))]
    DirsFileNotFound { config_file: PathBuf },
    #[snafu(display("unable to extract configuration"))]
    FigmentError { source: figment::Error },
}

type Result<T, E = ConfigError> = std::result::Result<T, E>;

impl Config {
    pub fn new(opts_conf: Option<PathBuf>) -> Result<Self> {
        let mut conf = Figment::new();

        // This is the configuration read at compile time
        // It is up to packagers to modify accordingly
        conf = conf.merge(providers::Toml::string(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../dirs.conf"
        ))));

        let uid = unsafe { libc::getuid() };
        let mut mode = if uid == 0 { Mode::Root } else { Mode::User };

        // Read the conf passed as opt
        conf = if let Some(config_file) = opts_conf {
            ensure!(config_file.exists(), DirsFileNotFoundSnafu { config_file });
            conf.merge(providers::Toml::file(config_file))
        } else if Path::new(CONF_FILENAME).exists() {
            mode = Mode::Project;
            // Read the conf in the current working directory
            conf.merge(providers::Toml::file(Path::new(CONF_FILENAME)))
        } else if uid != 0 {
            conf.merge(providers::Toml::string(
                &toml::to_string(&Dirs::new_user_dirs().context(DirectoriesSnafu {})?).unwrap(),
            ))
            // Configuration from /etc/rinit/rinit.conf is not read in user
            // mode because the values are completely
            // different.
        } else {
            let configdir = conf
                .find_value("configdir")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string();
            // read the system configuration
            conf.merge(providers::Toml::file(
                Path::new(&configdir).join(CONF_FILENAME),
            ))
        };

        // Read the configuration variables from the env
        conf = conf.merge(providers::Env::prefixed("RINIT_"));

        conf.join((
            "mode",
            match mode {
                Mode::Root => "Root",
                Mode::User => "User",
                Mode::Project => "Project",
            },
        ))
        .extract()
        .with_context(|_| FigmentSnafu)
    }
}

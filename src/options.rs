use crate::types::common::IOptions;

const DHALL_FILE_NAME: &str = "conf.dhall";

#[allow(clippy::result_large_err)]
pub fn get_ioptions() -> Result<IOptions, serde_dhall::Error> {
  serde_dhall::from_file(DHALL_FILE_NAME).parse()
}

use duckdb::{
    Connection, Result,
    core::{DataChunkHandle, Inserter, LogicalTypeId},
    duckdb_entrypoint_c_api,
    vscalar::{ScalarFunctionSignature, VScalar},
    vtab::arrow::WritableVector,
};
use std::error::Error;

/// `duckers_version()`: the version of the loaded extension.
struct VersionScalar;

impl VScalar for VersionScalar {
    type State = ();

    fn invoke(
        _state: &Self::State,
        input: &mut DataChunkHandle,
        output: &mut dyn WritableVector,
    ) -> Result<(), Box<dyn Error>> {
        let output = output.flat_vector();
        for i in 0..input.len() {
            output.insert(i, env!("CARGO_PKG_VERSION"));
        }
        Ok(())
    }

    fn signatures() -> Vec<ScalarFunctionSignature> {
        vec![ScalarFunctionSignature::exact(
            vec![],
            LogicalTypeId::Varchar.into(),
        )]
    }
}

#[duckdb_entrypoint_c_api]
pub unsafe fn extension_entrypoint(con: Connection) -> Result<(), Box<dyn Error>> {
    con.register_scalar_function::<VersionScalar>("duckers_version")?;
    Ok(())
}

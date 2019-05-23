use ::rsp2_integration_test::{CliTest, filetypes, resource, cli_test, Result};

#[ignore] // This test is expensive; use `cargo test -- --ignored` to run it!
#[test]
fn dynmat_lammps() -> Result<()> {
    let env = cli_test::Environment::init();
    CliTest::cargo_binary(&env, "rsp2")
        .arg("-c").arg(resource("defaults.yaml"))
        .arg("-c").arg(resource("sparse.yaml"))
        .arg(resource("001-a-relaxed-kcz.vasp"))
        .arg("-o").arg("out")
        .check_file::<filetypes::Dynmat>(
            "out/gamma-dynmat.npz".as_ref(),
            resource("sparse-out/gamma-dynmat.npz").as_ref(),
            filetypes::DynmatTolerances {
                rel_tol: 1e-9,
                abs_tol: 1e-9,
            },
        )
        .run()
}

// Test custom DispFns on the potentials implemented in Rust
#[ignore] // This test is expensive; use `cargo test -- --ignored` to run it!
#[test]
fn dynmat_rust() -> Result<()> {
    let env = cli_test::Environment::init();
    CliTest::cargo_binary(&env, "rsp2")
        .arg("-c").arg(resource("defaults.yaml"))
        .arg("-c").arg(resource("sparse-rust.yaml"))
        .arg(resource("001-a-relaxed-kcz.vasp"))
        .arg("-o").arg("out")
        .check_file::<filetypes::Dynmat>(
            "out/gamma-dynmat.npz".as_ref(),
            resource("sparse-out/gamma-dynmat.npz").as_ref(),
            filetypes::DynmatTolerances {
                rel_tol: 1e-9,
                abs_tol: 1e-9,
            },
        )
        .run()
}

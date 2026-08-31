//! Minimal facade adoption smoke example compiled by `cargo xtask adoption`.

use dol::prelude::*;

#[derive(Clone, Debug, dol::Model)]
#[dol(key = "example/adoption-user", name = "AdoptionUser")]
struct AdoptionUser {
    #[dol(identity)]
    id: u64,
    active: bool,
    score: i64,
    #[dol(optional)]
    nickname: Option<String>,
}

fn main() -> dol::core::diagnostic::Result<()> {
    let plan = Pipeline::<AdoptionUser>::from_model()
        .filter(
            AdoptionUser::active
                .eq(true)
                .and(AdoptionUser::score.ge(10)),
        )
        .select((AdoptionUser::id, AdoptionUser::nickname))
        .limit(25)
        .logical_plan()?;

    println!(
        "adoption plan: nodes={}, fingerprint={:?}",
        plan.nodes().len(),
        plan.fingerprint()
    );
    Ok(())
}

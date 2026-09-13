//! Exact vector-search adoption example.

#[cfg(feature = "ml")]
use dol::ml::{
    Distance, ExactSearchRequest, SearchLimits, Vector, VectorCandidate, VectorId, exact_search,
};

#[cfg(feature = "ml")]
fn main() -> dol::ml::Result<()> {
    let limits = SearchLimits::default();
    let request =
        ExactSearchRequest::new(Vector::new(vec![1.0, 0.0])?, Distance::Euclidean, 2, limits)?;
    let matches = exact_search(
        &request,
        [
            VectorCandidate::new(VectorId::new("near")?, Vector::new(vec![0.9, 0.1])?),
            VectorCandidate::new(VectorId::new("far")?, Vector::new(vec![0.0, 1.0])?),
        ],
        limits,
    )?;
    println!("nearest={}", matches[0].id().as_str());
    Ok(())
}

#[cfg(not(feature = "ml"))]
fn main() {
    eprintln!("run with `--features ml`");
}

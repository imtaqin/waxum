//! Voting in polls and reading their results. Votes are end-to-end
//! encrypted with the poll's message secret, so results are tallied here
//! from the vote messages in the session's chat store.

pub mod handlers;
pub mod models;
pub mod routes;

//! HTTP edge of the flows over courses (ADR-0093): the cards of the home page, the header and the
//! people of a course, and its lifecycle. Writes open the transaction, call exactly one service
//! function and commit; reads use a pooled connection.

mod cards;
mod header;
mod lifecycle;

use access::ActorSource;
use axum::extract::FromRef;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::LearningState;

pub fn router<S>() -> OpenApiRouter<S>
where
    S: ActorSource + Clone + 'static,
    LearningState: FromRef<S>,
{
    OpenApiRouter::new()
        .routes(routes!(cards::get_course_cards))
        .routes(routes!(header::get_course_info, header::update_course_info))
        .routes(routes!(header::get_people))
        .routes(routes!(lifecycle::publish_course))
        .routes(routes!(lifecycle::delete_course))
        .routes(routes!(lifecycle::list_templates))
        .routes(routes!(lifecycle::make_template))
        .routes(routes!(lifecycle::create_from_template))
}

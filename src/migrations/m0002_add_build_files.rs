//! Build files: whether an app keeps them in the media library, and the hash
//! of the token a pipeline uploads them with.

use laterite_core::sea_query;
use laterite_core::strata::*;
use sea_query::Alias;

pub struct Migration;

#[async_trait(?Send)]
impl laterite_core::Migration for Migration {
    fn name(&self) -> &str {
        "0002_add_build_files"
    }
    async fn up(&self, s: &mut Schema<'_>) -> CoreResult<()> {
        s.exec(
            Table::alter()
                .table(Alias::new("mobile_apps"))
                .add_column(
                    bool_col(Alias::new("files_in_library"))
                        .not_null()
                        .default(0),
                )
                .to_owned(),
        )
        .await?;
        s.exec(
            Table::alter()
                .table(Alias::new("mobile_apps"))
                .add_column(key_col(Alias::new("upload_token_hash")).null())
                .to_owned(),
        )
        .await
    }
}

//! The application's own conditions a policy waits on, by name: a policy
//! naming `location` applies only where the registered `location` dimension
//! says so.

use laterite_core::sea_query;
use laterite_core::strata::*;
use sea_query::Alias;

pub struct Migration;

#[async_trait(?Send)]
impl laterite_core::Migration for Migration {
    fn name(&self) -> &str {
        "0003_add_policy_dimensions"
    }
    async fn up(&self, s: &mut Schema<'_>) -> CoreResult<()> {
        s.exec(
            Table::alter()
                .table(Alias::new("mobile_policies"))
                .add_column(ColumnDef::new(Alias::new("dimensions")).text())
                .to_owned(),
        )
        .await
    }
    async fn down(&self, s: &mut Schema<'_>) -> CoreResult<()> {
        s.exec(
            Table::alter()
                .table(Alias::new("mobile_policies"))
                .drop_column(Alias::new("dimensions"))
                .to_owned(),
        )
        .await
    }
}

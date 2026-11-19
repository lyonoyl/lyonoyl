import type { MailAccountSummaryDto } from "@/shared/ipc/generated/MailAccountSummaryDto";
import type { WorkspaceOverviewDto } from "@/shared/ipc/generated/WorkspaceOverviewDto";

/**
 * The mailboxes, with one of them signing as the core kept its signature. Empty is none, which is
 * a mailbox with no signature at all, as the core sends one.
 */
export function signedAs(
  workspace: WorkspaceOverviewDto,
  accountId: string,
  signature: string,
): WorkspaceOverviewDto {
  return {
    ...workspace,
    accounts: workspace.accounts.map((account) =>
      account.id === accountId
        ? { ...account, signature: signature === "" ? undefined : signature }
        : account,
    ),
  };
}

/**
 * The mailboxes with this one among them: in its place when it is already here, and last when it
 * is not.
 *
 * The form a mailbox is added or corrected in knows nothing of its signature, so the mailbox it
 * hands back carries none: one already here keeps the signature it had, which the core kept too.
 */
export function withAccount(
  workspace: WorkspaceOverviewDto,
  account: MailAccountSummaryDto,
): WorkspaceOverviewDto {
  const here = workspace.accounts.some((one) => one.id === account.id);

  return {
    ...workspace,
    accounts: here
      ? workspace.accounts.map((one) =>
          one.id === account.id ? { ...account, signature: one.signature } : one,
        )
      : [...workspace.accounts, account],
  };
}

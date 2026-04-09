import UpstreamAccountsPageImpl, {
  SharedUpstreamAccountDetailDrawer as SharedUpstreamAccountDetailDrawerImpl,
} from "./UpstreamAccounts.page-impl";

export const SharedUpstreamAccountDetailDrawer =
  SharedUpstreamAccountDetailDrawerImpl;

export default function UpstreamAccountsPageCore() {
  return <UpstreamAccountsPageImpl />;
}

import { API_ENV_TO_INFO } from '_app/ApiProvider';
import Button from '_app/shared/button';
import Icon, { SuiIcons } from '_components/icon';
import { useAppSelector } from '_hooks';

import st from './RequestButton.module.scss';

function FaucetRequestButton() {
    const network = useAppSelector(({ app }) => app.apiEnv);
    const networkName = API_ENV_TO_INFO[network].name;
    return (
        <Button mode="primary">
            <Icon icon={SuiIcons.Download} className={st.icon} />
            Request {networkName} SUI Tokens
        </Button>
    );
}

export default FaucetRequestButton;

import { createApp } from "vue";
import App from "./App.vue";
import { installErrorLogging, setDetailedLogging } from "./lib/log";
import { settings } from "./lib/settings";
import "vue-echarts/style.css";
import "./styles.css";

installErrorLogging();
// The backend starts with normal logging; apply the saved choice.
if (settings.detailedLog) setDetailedLogging(true).catch(() => {});

createApp(App).mount("#app");

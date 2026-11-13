import { createApp } from 'vue';
import App from './App.vue';
import { startDesktop } from './desktop';
import { startStore } from './store';
import './styles/main.css';

createApp(App).mount('#app');
startStore();
startDesktop();

import { mount } from "svelte";
import App from "./App.svelte";
import "@fontsource-variable/manrope";
import "./styles.css";

mount(App, { target: document.getElementById("app")! });

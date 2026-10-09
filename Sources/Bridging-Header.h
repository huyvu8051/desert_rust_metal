#pragma once

void *baseline_create(void *view);
void baseline_render_frame(void *renderer);
void baseline_update_camera(void *renderer, float forward, float strafe, float look_dx, float look_dy, float delta_time);
void baseline_release(void *renderer);

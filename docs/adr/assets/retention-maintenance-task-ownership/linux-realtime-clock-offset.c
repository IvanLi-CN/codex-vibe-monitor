#define _GNU_SOURCE
#include <stdlib.h>
#include <sys/syscall.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>
/* Offset only process realtime; both clock domains continue to advance normally. */
static struct timespec initial_realtime;
__attribute__((constructor)) static void initialize_clock(void) {
    syscall(SYS_clock_gettime, CLOCK_REALTIME, &initial_realtime);
}
int clock_gettime(clockid_t clock_id, struct timespec *value) {
    int result = syscall(SYS_clock_gettime, clock_id, value);
    const char *epoch = getenv("CVM_TEST_REALTIME_EPOCH");
    if (!result && clock_id == CLOCK_REALTIME && epoch) {
        value->tv_sec += strtoll(epoch, NULL, 10) - initial_realtime.tv_sec;
        value->tv_nsec -= initial_realtime.tv_nsec;
        if (value->tv_nsec < 0) { value->tv_nsec += 1000000000; value->tv_sec--; }
    }
    return result;
}
int gettimeofday(struct timeval *value, void *zone) {
    (void)zone;
    struct timespec now;
    int result = clock_gettime(CLOCK_REALTIME, &now);
    if (!result) { value->tv_sec = now.tv_sec; value->tv_usec = now.tv_nsec / 1000; }
    return result;
}
time_t time(time_t *value) {
    struct timespec now;
    if (clock_gettime(CLOCK_REALTIME, &now)) return (time_t)-1;
    if (value) *value = now.tv_sec;
    return now.tv_sec;
}

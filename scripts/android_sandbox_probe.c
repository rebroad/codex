#define _GNU_SOURCE

#include <errno.h>
#include <fcntl.h>
#include <linux/audit.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <signal.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/prctl.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#ifndef LANDLOCK_CREATE_RULESET_VERSION
#define LANDLOCK_CREATE_RULESET_VERSION (1U << 0)
#endif

/* This tests kernel primitives only; it does not install a filesystem policy.
 */
struct child_setup {
  int error;
};

static int landlock_abi_version(void) {
#ifdef SYS_landlock_create_ruleset
  errno = 0;
  long abi = syscall(SYS_landlock_create_ruleset, NULL, 0,
                     LANDLOCK_CREATE_RULESET_VERSION);
  return abi < 0 ? 0 : (int)abi;
#else
  return 0;
#endif
}

static int send_listener(int socket_fd, int listener_fd, int setup_error) {
  struct child_setup setup = {.error = setup_error};
  struct iovec iov = {.iov_base = &setup, .iov_len = sizeof(setup)};
  char control[CMSG_SPACE(sizeof(int))];
  memset(control, 0, sizeof(control));

  struct msghdr message = {
      .msg_iov = &iov,
      .msg_iovlen = 1,
  };
  if (listener_fd >= 0) {
    message.msg_control = control;
    message.msg_controllen = sizeof(control);
    struct cmsghdr *header = CMSG_FIRSTHDR(&message);
    header->cmsg_level = SOL_SOCKET;
    header->cmsg_type = SCM_RIGHTS;
    header->cmsg_len = CMSG_LEN(sizeof(int));
    memcpy(CMSG_DATA(header), &listener_fd, sizeof(listener_fd));
    message.msg_controllen = sizeof(control);
  }
  return syscall(SYS_sendmsg, socket_fd, &message, 0) == (ssize_t)sizeof(setup)
             ? 0
             : -1;
}

static int receive_listener(int socket_fd, int *listener_fd, int *setup_error) {
  struct child_setup setup = {0};
  struct iovec iov = {.iov_base = &setup, .iov_len = sizeof(setup)};
  char control[CMSG_SPACE(sizeof(int))];
  memset(control, 0, sizeof(control));
  struct msghdr message = {
      .msg_iov = &iov,
      .msg_iovlen = 1,
      .msg_control = control,
      .msg_controllen = sizeof(control),
  };

  ssize_t size = recvmsg(socket_fd, &message, 0);
  if (size != (ssize_t)sizeof(setup)) {
    return -1;
  }
  *listener_fd = -1;
  *setup_error = setup.error;
  for (struct cmsghdr *header = CMSG_FIRSTHDR(&message); header != NULL;
       header = CMSG_NXTHDR(&message, header)) {
    if (header->cmsg_level == SOL_SOCKET && header->cmsg_type == SCM_RIGHTS &&
        header->cmsg_len >= CMSG_LEN(sizeof(int))) {
      memcpy(listener_fd, CMSG_DATA(header), sizeof(*listener_fd));
      break;
    }
  }
  return 0;
}

static int install_openat_listener(void) {
  struct sock_filter filter[] = {
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
               (uint32_t)offsetof(struct seccomp_data, arch)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, AUDIT_ARCH_AARCH64, 1, 0),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS,
               (uint32_t)offsetof(struct seccomp_data, nr)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_openat, 0, 1),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_USER_NOTIF),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
  };
  struct sock_fprog program = {
      .len = (unsigned short)(sizeof(filter) / sizeof(filter[0])),
      .filter = filter,
  };

  if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0) {
    return -1;
  }
  return (int)syscall(SYS_seccomp, SECCOMP_SET_MODE_FILTER,
                      SECCOMP_FILTER_FLAG_NEW_LISTENER, &program);
}

static int wait_for_child(pid_t child) {
  int status = 0;
  while (waitpid(child, &status, 0) < 0) {
    if (errno != EINTR) {
      return -1;
    }
  }
  return WIFEXITED(status) ? WEXITSTATUS(status) : -1;
}

static void stop_child(pid_t child) {
  (void)kill(child, SIGKILL);
  (void)wait_for_child(child);
}

static int probe_user_notification(int *supported, int *addfd_send) {
  int sockets[2];
  if (socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sockets) != 0) {
    return -1;
  }

  pid_t child = fork();
  if (child < 0) {
    close(sockets[0]);
    close(sockets[1]);
    return -1;
  }
  if (child == 0) {
    close(sockets[0]);
    int listener = install_openat_listener();
    int setup_error = listener < 0 ? errno : 0;
    if (send_listener(sockets[1], listener, setup_error) != 0) {
      _exit(70);
    }
    if (listener < 0) {
      _exit(0);
    }
    close(listener);
    close(sockets[1]);

    int fd = (int)syscall(SYS_openat, AT_FDCWD, "/dev/null",
                          O_RDONLY | O_CLOEXEC, 0);
    if (fd < 0) {
      _exit(71);
    }
    char byte;
    ssize_t count = read(fd, &byte, sizeof(byte));
    close(fd);
    _exit(count == 0 ? 0 : 72);
  }

  close(sockets[1]);
  int listener = -1;
  int setup_error = 0;
  if (receive_listener(sockets[0], &listener, &setup_error) != 0) {
    close(sockets[0]);
    stop_child(child);
    return -1;
  }
  close(sockets[0]);
  if (listener < 0) {
    (void)wait_for_child(child);
    *supported = 0;
    *addfd_send = 0;
    return setup_error == 0 ? -1 : 0;
  }

  int source_fd = open("/dev/null", O_RDONLY | O_CLOEXEC);
  struct seccomp_notif notification;
  memset(&notification, 0, sizeof(notification));
  if (source_fd < 0 ||
      ioctl(listener, SECCOMP_IOCTL_NOTIF_RECV, &notification) != 0) {
    if (source_fd >= 0) {
      close(source_fd);
    }
    close(listener);
    stop_child(child);
    return -1;
  }
  *supported = 1;

  struct seccomp_notif_addfd addfd = {
      .id = notification.id,
      .flags = SECCOMP_ADDFD_FLAG_SEND,
      .srcfd = (uint32_t)source_fd,
      .newfd = 0,
      .newfd_flags = O_CLOEXEC,
  };
  int injected_fd = ioctl(listener, SECCOMP_IOCTL_NOTIF_ADDFD, &addfd);
  close(source_fd);
  if (injected_fd < 0) {
    struct seccomp_notif_resp response = {
        .id = notification.id,
        .val = 0,
        .error = -ENOSYS,
        .flags = 0,
    };
    (void)ioctl(listener, SECCOMP_IOCTL_NOTIF_SEND, &response);
    *addfd_send = 0;
  } else {
    *addfd_send = wait_for_child(child) == 0;
  }

  close(listener);
  if (injected_fd < 0) {
    (void)wait_for_child(child);
  }
  return 0;
}

int main(void) {
  int landlock_abi = landlock_abi_version();
  int user_notification = 0;
  int addfd_send = 0;
  if (probe_user_notification(&user_notification, &addfd_send) != 0) {
    perror("seccomp user-notification probe failed");
    return 1;
  }

  printf("{\"filesystem_policy_enforced\":false,\"landlock_supported\":%s,"
         "\"landlock_abi\":",
         landlock_abi > 0 ? "true" : "false");
  if (landlock_abi > 0) {
    printf("%d", landlock_abi);
  } else {
    printf("null");
  }
  printf(",\"seccomp_user_notif_supported\":%s,\"seccomp_addfd_send_"
         "supported\":%s}\n",
         user_notification ? "true" : "false", addfd_send ? "true" : "false");
  return 0;
}
